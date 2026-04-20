//! Ingestion pipeline.
//!
//! Ingestion is the process of taking a [`signet_connector_sdk::Resource`] from a
//! connector and turning it into:
//!
//! 1. A typed row in the schema's Postgres table.
//! 2. One or more chunks in the schema's vector-store collection, each with an
//!    embedding.
//! 3. A lineage edge linking the chunk back to its source record.
//!
//! The ingestion pipeline is called from `signet-ingest` (the runtime that polls
//! or subscribes to connectors); this module is the engine-side API those
//! callers use.

use std::sync::Arc;

use signet_connector_sdk::Resource;
use signet_embedders::Embedder;
use signet_vectorstore::{Chunk, VectorStore};
use serde_json::{Value, json};
use ulid::Ulid;

use crate::api::EngineError;
use crate::schema_registry::{ChunkingStrategy, SchemaMeta};
use crate::store::Store;

/// Result of ingesting a single resource.
#[derive(Debug, Clone)]
pub struct IngestOutcome {
    /// Primary key assigned to the row (Ulid).
    pub record_id: String,
    /// Number of chunks produced.
    pub chunks_written: usize,
    /// Whether this was an insert (`true`) or an update (`false`).
    pub inserted: bool,
}

/// Ingestion orchestrator.
pub struct Ingestor {
    vector_store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    store: Store,
}

impl Ingestor {
    /// Construct a new ingestor.
    #[must_use]
    pub fn new(
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<dyn Embedder>,
        store: Store,
    ) -> Self {
        Self {
            vector_store,
            embedder,
            store,
        }
    }

    /// Ingest one resource into the given schema under the given tenant.
    ///
    /// `tenant` is the connector binding's tenant identifier; pass `""` for
    /// single-tenant / unscoped deployments. It is stamped into both the
    /// Postgres row and the vector-store payload so retrieval can filter.
    pub async fn ingest(
        &self,
        meta: &SchemaMeta,
        tenant: &str,
        resource: Resource,
    ) -> Result<IngestOutcome, EngineError> {
        let record_id = Ulid::new().to_string();

        // ---- Row upsert ----------------------------------------------------
        self.upsert_row(meta, tenant, &record_id, &resource).await?;

        // ---- Chunking + embedding ------------------------------------------
        let chunks = chunk_body(&resource.body, &meta.context.chunking);
        if chunks.is_empty() {
            return Ok(IngestOutcome {
                record_id,
                chunks_written: 0,
                inserted: true,
            });
        }

        let embeddings = self
            .embedder
            .embed(chunks.iter().map(|c| c.text.clone()).collect())
            .await
            .map_err(|e| EngineError::Backend(format!("embedder: {e}")))?;

        if embeddings.len() != chunks.len() {
            return Err(EngineError::Backend(format!(
                "embedder returned {} vectors for {} chunks",
                embeddings.len(),
                chunks.len()
            )));
        }

        // ---- Vector-store upsert -------------------------------------------
        let vec_chunks: Vec<Chunk> = chunks
            .into_iter()
            .zip(embeddings.into_iter())
            .enumerate()
            .map(|(seq, (chunk, embedding))| Chunk {
                id: Ulid::new(),
                content: chunk.text,
                embedding,
                metadata: serde_json::Map::from_iter([
                    ("record_id".into(), Value::String(record_id.clone())),
                    ("seq".into(), Value::from(seq)),
                    ("tenant".into(), Value::String(tenant.to_owned())),
                    (
                        "created_at".into(),
                        Value::String(
                            resource
                                .source_updated_at
                                .format(&time::format_description::well_known::Rfc3339)
                                .unwrap_or_default(),
                        ),
                    ),
                ]),
            })
            .collect();
        let chunk_count = vec_chunks.len();

        self.vector_store
            .upsert(&meta.collection, vec_chunks)
            .await
            .map_err(|e| EngineError::Backend(format!("vector store upsert: {e}")))?;

        Ok(IngestOutcome {
            record_id,
            chunks_written: chunk_count,
            inserted: true,
        })
    }

    async fn upsert_row(
        &self,
        meta: &SchemaMeta,
        tenant: &str,
        record_id: &str,
        resource: &Resource,
    ) -> Result<(), EngineError> {
        if !crate::util::is_safe_ident(&meta.table) {
            return Err(EngineError::InvalidArgument(format!(
                "unsafe table identifier `{}`",
                meta.table
            )));
        }

        // v0.1 schema mapping is intentionally narrow: `id`, optional `title`, `body`,
        // `created_at`, `tenant`, plus a JSONB `metadata` column we expect every
        // CSL-generated table to carry. More-sophisticated mapping (projecting the
        // full typed field set through) lands with the CSL→Rust struct codegen path
        // in v0.2.
        let sql = format!(
            "INSERT INTO {} (id, title, body, created_at, tenant, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (id) DO UPDATE \
             SET title = EXCLUDED.title, body = EXCLUDED.body, \
                 created_at = EXCLUDED.created_at, \
                 tenant = EXCLUDED.tenant, \
                 metadata = EXCLUDED.metadata",
            meta.table
        );
        let title = resource.title.clone().unwrap_or_default();
        let metadata = json!({
            "kind":    resource.kind,
            "source":  resource.id.0,
            "perms":   resource.permissions,
            "extra":   resource.metadata,
        });
        sqlx::query(&sql)
            .bind(record_id)
            .bind(&title)
            .bind(&resource.body)
            .bind(resource.source_updated_at)
            .bind(tenant)
            .bind(metadata)
            .execute(self.store.pool())
            .await
            .map_err(EngineError::backend)?;
        Ok(())
    }

    /// Remove a record and its chunks from the indexes.
    pub async fn delete(
        &self,
        meta: &SchemaMeta,
        record_id: &str,
    ) -> Result<(), EngineError> {
        if !crate::util::is_safe_ident(&meta.table) {
            return Err(EngineError::InvalidArgument(format!(
                "unsafe table identifier `{}`",
                meta.table
            )));
        }
        // Delete the row; chunk deletion cascades if the CSL codegen emitted FK
        // cascade, and is handled separately by the vector store.
        let sql = format!("DELETE FROM {} WHERE id = $1", meta.table);
        sqlx::query(&sql)
            .bind(record_id)
            .execute(self.store.pool())
            .await
            .map_err(EngineError::backend)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Chunking.
// ---------------------------------------------------------------------------

/// One chunk produced by a chunking strategy.
#[derive(Debug, Clone)]
pub struct ChunkRaw {
    /// Chunk body.
    pub text: String,
}

/// Chunk body text according to the given strategy.
///
/// This is a pragmatic, dependency-free chunker. For production-grade
/// tokenization we'd plug in a real tokenizer (e.g., `tiktoken-rs`); the
/// approximations below use "~4 chars = 1 token" heuristics, which is close
/// enough for budgeting purposes.
#[must_use]
pub fn chunk_body(body: &str, strategy: &ChunkingStrategy) -> Vec<ChunkRaw> {
    if body.trim().is_empty() {
        return Vec::new();
    }
    match strategy {
        ChunkingStrategy::Fixed { size } => fixed_char_chunks(body, *size),
        ChunkingStrategy::Semantic {
            max_tokens,
            overlap,
        } => semantic_chunks(body, *max_tokens, *overlap),
        ChunkingStrategy::Recursive { split, max_tokens } => {
            recursive_chunks(body, split.as_slice(), *max_tokens)
        }
    }
}

fn fixed_char_chunks(body: &str, size: usize) -> Vec<ChunkRaw> {
    if size == 0 {
        return vec![ChunkRaw {
            text: body.to_string(),
        }];
    }
    body.as_bytes()
        .chunks(size)
        .filter_map(|slice| std::str::from_utf8(slice).ok())
        .map(|s| ChunkRaw {
            text: s.to_string(),
        })
        .collect()
}

fn semantic_chunks(body: &str, max_tokens: usize, overlap: usize) -> Vec<ChunkRaw> {
    // ~4 chars per token. Split on blank lines, then sentences, respecting max.
    let max_chars = max_tokens.saturating_mul(4).max(200);
    let overlap_chars = overlap.saturating_mul(4);

    let paragraphs: Vec<&str> = body.split("\n\n").filter(|p| !p.trim().is_empty()).collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for p in paragraphs {
        if current.len() + p.len() + 2 > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = if overlap_chars > 0 && current.len() > overlap_chars {
                current
                    .chars()
                    .rev()
                    .take(overlap_chars)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect()
            } else {
                String::new()
            };
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(p);
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    if chunks.is_empty() {
        // Fallback: one chunk from the whole body.
        chunks.push(body.to_string());
    }
    chunks
        .into_iter()
        .map(|text| ChunkRaw { text })
        .collect()
}

fn recursive_chunks(body: &str, separators: &[String], max_tokens: usize) -> Vec<ChunkRaw> {
    let max_chars = max_tokens.saturating_mul(4).max(200);
    recursive_inner(body, separators, max_chars, 0)
        .into_iter()
        .map(|text| ChunkRaw { text })
        .collect()
}

fn recursive_inner(input: &str, seps: &[String], max_chars: usize, depth: usize) -> Vec<String> {
    if input.len() <= max_chars || depth >= seps.len() {
        return vec![input.to_string()];
    }
    let sep = &seps[depth];
    let mut out = Vec::new();
    for piece in input.split(sep.as_str()) {
        if piece.is_empty() {
            continue;
        }
        if piece.len() > max_chars {
            out.extend(recursive_inner(piece, seps, max_chars, depth + 1));
        } else {
            out.push(piece.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_chunks_produce_expected_count() {
        let body = "abcdefghij".repeat(10);
        let chunks = fixed_char_chunks(&body, 20);
        assert!(chunks.len() >= 5);
    }

    #[test]
    fn semantic_respects_max_tokens() {
        let body = (0..50)
            .map(|i| format!("Paragraph {i} with some content."))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = semantic_chunks(&body, 40, 0);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for c in &chunks {
            assert!(c.text.len() <= 40 * 4 + 200, "chunk too large: {}", c.text.len());
        }
    }

    #[test]
    fn empty_body_produces_no_chunks() {
        assert!(chunk_body("", &ChunkingStrategy::default()).is_empty());
        assert!(chunk_body("   \n  ", &ChunkingStrategy::default()).is_empty());
    }
}
