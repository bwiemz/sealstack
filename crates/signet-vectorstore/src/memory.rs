//! In-process vector store.
//!
//! Backed by a `DashMap<collection, Vec<Chunk>>`. Search is linear — we compute
//! cosine similarity against every chunk and return the top-k. Fine up to ~10k
//! chunks per collection; switch to [`crate::qdrant::QdrantStore`] beyond that.
//!
//! Filtering is applied post-search against the chunk `metadata` map. Only
//! shallow equality on string / number / bool fields is supported; nested or
//! array filters are rejected.

use std::cmp::Ordering;

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use crate::{SignetError, SignetResult, Chunk, Distance, SearchResult, VectorStore};

/// An in-process vector store.
#[derive(Default)]
pub struct InMemoryStore {
    collections: DashMap<String, CollectionState>,
}

struct CollectionState {
    dims: usize,
    distance: Distance,
    chunks: Vec<Chunk>,
}

impl InMemoryStore {
    /// Construct an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure(&self, name: &str, dims: usize, distance: Distance) -> SignetResult<()> {
        if let Some(existing) = self.collections.get(name) {
            if existing.dims != dims {
                return Err(SignetError::Validation(format!(
                    "collection `{name}` exists with dims={}, requested {}",
                    existing.dims, dims
                )));
            }
            return Ok(());
        }
        self.collections.insert(
            name.to_owned(),
            CollectionState {
                dims,
                distance,
                chunks: Vec::new(),
            },
        );
        tracing::debug!(collection = %name, dims, "created in-memory collection");
        Ok(())
    }
}

#[async_trait]
impl VectorStore for InMemoryStore {
    fn kind(&self) -> &'static str {
        "memory"
    }

    async fn ensure_collection(&self, name: &str, dims: usize) -> SignetResult<()> {
        self.ensure(name, dims, Distance::default())
    }

    async fn ensure_collection_spec(&self, spec: &crate::CollectionSpec) -> SignetResult<()> {
        self.ensure(&spec.name, spec.dims, spec.distance)
    }

    async fn upsert(&self, collection: &str, chunks: Vec<Chunk>) -> SignetResult<()> {
        let mut entry = self.collections.get_mut(collection).ok_or_else(|| {
            SignetError::NotFound(format!("collection `{collection}` not found"))
        })?;
        for new_chunk in chunks {
            if new_chunk.embedding.len() != entry.dims {
                return Err(SignetError::Validation(format!(
                    "embedding dim {} != collection dims {}",
                    new_chunk.embedding.len(),
                    entry.dims
                )));
            }
            // Upsert by id.
            if let Some(existing) = entry.chunks.iter_mut().find(|c| c.id == new_chunk.id) {
                *existing = new_chunk;
            } else {
                entry.chunks.push(new_chunk);
            }
        }
        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        query_vec: Vec<f32>,
        top_k: usize,
        filter: Option<Value>,
    ) -> SignetResult<Vec<SearchResult>> {
        let entry = self.collections.get(collection).ok_or_else(|| {
            SignetError::NotFound(format!("collection `{collection}` not found"))
        })?;
        if query_vec.len() != entry.dims {
            return Err(SignetError::Validation(format!(
                "query vector dim {} != collection dims {}",
                query_vec.len(),
                entry.dims
            )));
        }

        let filter_obj = filter.as_ref().and_then(Value::as_object);

        let mut scored: Vec<(f32, &Chunk)> = entry
            .chunks
            .iter()
            .filter(|c| match filter_obj {
                None => true,
                Some(obj) => matches_filter(&c.metadata, obj),
            })
            .map(|c| {
                let s = match entry.distance {
                    Distance::Cosine => cosine_similarity(&query_vec, &c.embedding),
                    Distance::Dot => dot(&query_vec, &c.embedding),
                    Distance::Euclidean => -euclidean(&query_vec, &c.embedding),
                };
                (s, c)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        scored.truncate(top_k);

        Ok(scored
            .into_iter()
            .map(|(s, c)| SearchResult {
                id: c.id,
                score: s,
                content: c.content.clone(),
                metadata: c.metadata.clone(),
            })
            .collect())
    }

    async fn delete(&self, collection: &str, ids: Vec<ulid::Ulid>) -> SignetResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut entry = self.collections.get_mut(collection).ok_or_else(|| {
            SignetError::NotFound(format!("collection `{collection}` not found"))
        })?;
        let id_set: std::collections::HashSet<ulid::Ulid> = ids.into_iter().collect();
        entry.chunks.retain(|c| !id_set.contains(&c.id));
        Ok(())
    }

    async fn count(&self, collection: &str) -> SignetResult<u64> {
        let entry = self.collections.get(collection).ok_or_else(|| {
            SignetError::NotFound(format!("collection `{collection}` not found"))
        })?;
        Ok(entry.chunks.len() as u64)
    }

    async fn drop_collection(&self, name: &str) -> SignetResult<()> {
        self.collections.remove(name);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Similarity math
// ---------------------------------------------------------------------------

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn norm(a: &[f32]) -> f32 {
    dot(a, a).sqrt()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let denom = norm(a) * norm(b);
    if denom == 0.0 {
        0.0
    } else {
        (dot(a, b) / denom).clamp(-1.0, 1.0)
    }
}

fn euclidean(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

// ---------------------------------------------------------------------------
// Filter matching
// ---------------------------------------------------------------------------

/// Shallow metadata equality matcher.
///
/// Supports: `{ "key": "value" }` → `metadata.key == "value"`, `{ "key": 42 }`,
/// `{ "key": true }`. For each filter entry, the chunk metadata must contain
/// the same value at the same key. Arrays and nested objects are rejected.
fn matches_filter(
    metadata: &serde_json::Map<String, Value>,
    filter: &serde_json::Map<String, Value>,
) -> bool {
    for (k, v) in filter {
        let Some(actual) = metadata.get(k) else {
            return false;
        };
        match v {
            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
                if actual != v {
                    return false;
                }
            }
            // Unsupported: arrays, objects. Treat as no-match to be safe.
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, vec: Vec<f32>, tag: &str) -> Chunk {
        let mut meta = serde_json::Map::new();
        meta.insert("tag".into(), Value::String(tag.into()));
        Chunk {
            id: ulid::Ulid::from_string(
                &format!("{id:0>26}").to_uppercase().replace('I', "J").replace('L', "J"),
            )
            .unwrap_or_else(|_| ulid::Ulid::new()),
            content: format!("content-{id}"),
            embedding: vec,
            metadata: meta,
        }
    }

    #[tokio::test]
    async fn ensure_and_upsert_and_search() {
        let s = InMemoryStore::new();
        s.ensure_collection("c", 3).await.unwrap();
        s.upsert(
            "c",
            vec![
                chunk("a", vec![1.0, 0.0, 0.0], "x"),
                chunk("b", vec![0.0, 1.0, 0.0], "y"),
                chunk("c", vec![0.0, 0.0, 1.0], "x"),
            ],
        )
        .await
        .unwrap();
        let hits = s
            .search("c", vec![0.9, 0.1, 0.0], 2, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].score > hits[1].score);
    }

    #[tokio::test]
    async fn filter_post_search() {
        let s = InMemoryStore::new();
        s.ensure_collection("c", 3).await.unwrap();
        s.upsert(
            "c",
            vec![
                chunk("a", vec![1.0, 0.0, 0.0], "x"),
                chunk("b", vec![0.0, 1.0, 0.0], "y"),
                chunk("c", vec![0.0, 0.0, 1.0], "x"),
            ],
        )
        .await
        .unwrap();
        let hits = s
            .search(
                "c",
                vec![1.0, 1.0, 1.0],
                5,
                Some(serde_json::json!({ "tag": "x" })),
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        for h in &hits {
            assert_eq!(h.metadata["tag"], Value::String("x".into()));
        }
    }

    #[tokio::test]
    async fn dims_mismatch_is_validation_error() {
        let s = InMemoryStore::new();
        s.ensure_collection("c", 3).await.unwrap();
        let bad = chunk("x", vec![1.0, 0.0], "z"); // 2 dims, not 3
        let err = s.upsert("c", vec![bad]).await.unwrap_err();
        assert!(matches!(err, SignetError::Validation(_)));
    }

    #[test]
    fn cosine_math_is_correct() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]) - 0.0).abs() < 1e-6);
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) - (-1.0)).abs() < 1e-6);
    }
}
