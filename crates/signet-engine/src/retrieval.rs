//! Hybrid retrieval: vector + BM25 + freshness + rerank.
//!
//! # Pipeline
//!
//! ```text
//! query ──▶ embed ──┐
//!                   ▼
//!            vector.search (k = candidate_k)     ┐
//!                                                 ├─▶ RRF merge ──▶ freshness
//!            bm25.search   (k = candidate_k)     ┘                    ▼
//!                                                                   rerank
//!                                                                     ▼
//!                                                                  top_k
//! ```
//!
//! # Candidate set sizing
//!
//! We retrieve `candidate_k` hits from each backend independently (default 64),
//! merge them via Reciprocal Rank Fusion, apply freshness decay, then rerank.
//! This deliberately over-retrieves; reranking is the accuracy-critical stage
//! and benefits from seeing more diverse candidates than the final `top_k`.
//!
//! # BM25 backend
//!
//! v0.1 uses Postgres full-text search (`ts_vector`/`ts_rank_cd`) as the BM25
//! stand-in. It is not strictly BM25 but is close enough for dev and small
//! production deployments. A Tantivy-backed implementation is planned for v0.2
//! for deployments that need true BM25 scoring.

use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use time::OffsetDateTime;

use signet_embedders::Embedder;
use signet_vectorstore::{SearchResult as VecHit, VectorStore};

use crate::api::EngineError;
use crate::config::RetrievalConfig;
use crate::freshness::decay_factor;
use crate::rerank::{RerankCandidate, Reranker};
use crate::schema_registry::SchemaMeta;
use crate::store::Store;

/// Candidate before fusion.
#[derive(Clone, Debug)]
struct RawCandidate {
    id: String,
    score: f32,
    content: String,
    created_at: Option<OffsetDateTime>,
}

/// One output hit from retrieval (pre-engine-response stage).
#[derive(Clone, Debug)]
pub struct RetrievedHit {
    /// Primary key of the source record.
    pub id: String,
    /// Fused, decayed, reranked score.
    pub score: f32,
    /// Chunk excerpt.
    pub excerpt: String,
}

/// Retrieval orchestrator.
pub struct Retriever {
    config: RetrievalConfig,
    vector_store: Arc<dyn VectorStore>,
    embedder: Arc<dyn Embedder>,
    store: Store,
    reranker: Arc<dyn Reranker>,
}

impl Retriever {
    /// Construct a new retriever.
    #[must_use]
    pub fn new(
        config: RetrievalConfig,
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<dyn Embedder>,
        store: Store,
        reranker: Arc<dyn Reranker>,
    ) -> Self {
        Self {
            config,
            vector_store,
            embedder,
            store,
            reranker,
        }
    }

    /// Run hybrid retrieval for one query.
    ///
    /// `filters` is a free-form facet filter object; unknown keys are ignored.
    /// `tenant` is the caller's tenant identifier; passing `""` scopes the
    /// query to rows whose `tenant` column is `NULL` or empty, which matches
    /// single-tenant dev deployments.
    pub async fn search(
        &self,
        meta: &SchemaMeta,
        query: &str,
        top_k: usize,
        filters: &Value,
        tenant: &str,
    ) -> Result<Vec<RetrievedHit>, EngineError> {
        let candidate_k = self.config.candidate_k;
        let alpha = meta
            .hybrid_alpha
            .unwrap_or(self.config.default_hybrid_alpha)
            .clamp(0.0, 1.0);

        // ---- Embedding -----------------------------------------------------
        let t_embed = Instant::now();
        let embedding = self
            .embedder
            .embed(vec![query.to_string()])
            .await
            .map_err(|e| EngineError::Backend(format!("embedder: {e}")))?
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::Backend("embedder returned zero vectors".into()))?;
        tracing::debug!(ms = t_embed.elapsed().as_millis() as u64, "embed");

        // ---- Parallel vector + BM25 ---------------------------------------
        let vector_fut =
            self.vector_search(&meta.collection, embedding, candidate_k, filters, tenant);
        let bm25_fut = self.bm25_search(meta, query, candidate_k, filters, tenant);

        let (vector_hits, bm25_hits) = tokio::join!(vector_fut, bm25_fut);
        let vector_hits = vector_hits.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "vector search failed; proceeding with BM25 only");
            Vec::new()
        });
        let bm25_hits = bm25_hits.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "BM25 search failed; proceeding with vector only");
            Vec::new()
        });

        if vector_hits.is_empty() && bm25_hits.is_empty() {
            return Ok(Vec::new());
        }

        // ---- Fuse ----------------------------------------------------------
        let fused = rrf_fuse(&vector_hits, &bm25_hits, alpha);

        // ---- Freshness decay ----------------------------------------------
        let now = OffsetDateTime::now_utc();
        let decayed: Vec<RawCandidate> = fused
            .into_iter()
            .map(|mut c| {
                if let Some(ts) = c.created_at {
                    let age = (now - ts).whole_seconds();
                    c.score *= decay_factor(&meta.context.freshness_decay, age);
                }
                c
            })
            .collect();

        // ---- Rerank --------------------------------------------------------
        let to_rerank: Vec<RerankCandidate> = decayed
            .iter()
            .take(candidate_k)
            .map(|c| RerankCandidate {
                id: c.id.clone(),
                text: c.content.clone(),
                prior_score: c.score,
            })
            .collect();

        let reranked = self.reranker.rerank(query, &to_rerank).await?;

        // Align scores + sort desc.
        let mut final_hits: Vec<RetrievedHit> = decayed
            .into_iter()
            .zip(reranked.iter())
            .map(|(raw, rr)| RetrievedHit {
                id: raw.id,
                score: rr.score,
                excerpt: raw.content,
            })
            .collect();
        final_hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        final_hits.truncate(top_k);

        Ok(final_hits)
    }

    // ---- Individual backends ---------------------------------------------

    async fn vector_search(
        &self,
        collection: &str,
        query_vec: Vec<f32>,
        k: usize,
        _filters: &Value,
        tenant: &str,
    ) -> Result<Vec<RawCandidate>, EngineError> {
        // Build a tenant filter. Vector stores that honor it will apply it
        // natively; the in-memory store falls back to post-filter matching on
        // the payload `tenant` key.
        //
        // TODO: map `_filters` into the vector store's native filter language.
        // For v0.1 we only thread the tenant facet.
        let filter = (!tenant.is_empty()).then(|| {
            serde_json::json!({
                "tenant": tenant,
            })
        });
        let hits: Vec<VecHit> = self
            .vector_store
            .search(collection, query_vec, k, filter)
            .await
            .map_err(|e| EngineError::Backend(format!("vector store: {e}")))?;
        Ok(hits.into_iter().map(vec_hit_to_raw).collect())
    }

    async fn bm25_search(
        &self,
        meta: &SchemaMeta,
        query: &str,
        k: usize,
        _filters: &Value,
        tenant: &str,
    ) -> Result<Vec<RawCandidate>, EngineError> {
        // Postgres ts_vector search. Table name and column names both come
        // from parsed CSL identifiers — trusted — but we still guard them
        // with `is_safe_ident` defense-in-depth.
        if !crate::util::is_safe_ident(&meta.table) {
            return Err(EngineError::InvalidArgument(format!(
                "unsafe table identifier `{}`",
                meta.table
            )));
        }

        // Build the concat expression over every `@chunked` field the schema
        // declares. Empty `chunked_fields` makes a match impossible, so we
        // short-circuit instead of generating `to_tsvector('english','')`.
        if meta.chunked_fields.is_empty() {
            tracing::debug!(
                schema = %meta.name,
                "no chunked_fields declared; skipping BM25 search",
            );
            return Ok(Vec::new());
        }
        for col in &meta.chunked_fields {
            if !crate::util::is_safe_ident(col) {
                return Err(EngineError::InvalidArgument(format!(
                    "unsafe chunked column `{col}` in schema `{}.{}`",
                    meta.namespace, meta.name
                )));
            }
        }

        // `coalesce(col1,'') || ' ' || coalesce(col2,'') || ...`
        let concat_expr = meta
            .chunked_fields
            .iter()
            .map(|c| format!("coalesce({c},'')"))
            .collect::<Vec<_>>()
            .join(" || ' ' || ");

        // The first `@chunked` field supplies the excerpt body when the
        // record has one; all subsequent fields feed the match score but
        // aren't echoed back as the excerpt.
        let first_col = &meta.chunked_fields[0];

        // Tenant filter: `coalesce(tenant,'') = $3`. Matches rows that either
        // carry an explicit tenant equal to the caller's, or have no tenant
        // set. Empty-string tenant means "default tenant" and only matches
        // rows without a tenant set — preserving isolation in mixed
        // deployments.
        let table = &meta.table;
        let sql = format!(
            "SELECT id::text AS id, \
                    COALESCE({first_col}, '') AS excerpt, \
                    created_at, \
                    ts_rank_cd(to_tsvector('english', {concat_expr}), \
                               plainto_tsquery('english', $1)) AS score \
             FROM {table} \
             WHERE to_tsvector('english', {concat_expr}) \
                   @@ plainto_tsquery('english', $1) \
               AND coalesce(tenant, '') = $3 \
             ORDER BY score DESC \
             LIMIT $2"
        );

        let rows: Vec<(String, String, Option<OffsetDateTime>, f32)> = sqlx::query_as(&sql)
            .bind(query)
            .bind(i64::try_from(k).unwrap_or(64))
            .bind(tenant)
            .fetch_all(self.store.pool())
            .await
            .map_err(EngineError::backend)?;

        Ok(rows
            .into_iter()
            .map(|(id, excerpt, ts, score)| RawCandidate {
                id,
                score,
                content: excerpt,
                created_at: ts,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn vec_hit_to_raw(h: VecHit) -> RawCandidate {
    let created_at = h
        .metadata
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(|s| OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok());
    RawCandidate {
        id: h.id.to_string(),
        score: h.score,
        content: h.content,
        created_at,
    }
}

/// Reciprocal Rank Fusion with a linear blend.
///
/// For each source, each candidate gets score `1 / (k + rank)` where `k = 60`.
/// The final score is `alpha * vector_rrf + (1 - alpha) * bm25_rrf`.
///
/// References: Cormack et al. 2009 ("Reciprocal Rank Fusion Outperforms
/// Condorcet and Individual Rank Learning Methods", SIGIR 2009).
fn rrf_fuse(vec_hits: &[RawCandidate], bm25_hits: &[RawCandidate], alpha: f32) -> Vec<RawCandidate> {
    const K: f32 = 60.0;
    use std::collections::HashMap;
    let mut map: HashMap<String, RawCandidate> = HashMap::new();
    let mut score: HashMap<String, f32> = HashMap::new();

    for (rank, c) in vec_hits.iter().enumerate() {
        let s = alpha * (1.0 / (K + rank as f32 + 1.0));
        *score.entry(c.id.clone()).or_insert(0.0) += s;
        map.entry(c.id.clone()).or_insert_with(|| c.clone());
    }
    for (rank, c) in bm25_hits.iter().enumerate() {
        let s = (1.0 - alpha) * (1.0 / (K + rank as f32 + 1.0));
        *score.entry(c.id.clone()).or_insert(0.0) += s;
        map.entry(c.id.clone()).or_insert_with(|| c.clone());
    }

    let mut fused: Vec<RawCandidate> = map
        .into_iter()
        .map(|(id, mut c)| {
            c.score = *score.get(&id).unwrap_or(&0.0);
            c
        })
        .collect();
    fused.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: &str, score: f32) -> RawCandidate {
        RawCandidate {
            id: id.into(),
            score,
            content: String::new(),
            created_at: None,
        }
    }

    #[test]
    fn rrf_merges_two_lists() {
        let v = vec![raw("a", 0.9), raw("b", 0.8), raw("c", 0.7)];
        let b = vec![raw("c", 0.95), raw("a", 0.6)];
        let fused = rrf_fuse(&v, &b, 0.5);
        let top_id = fused.first().unwrap().id.clone();
        // `a` appears #1 in vector and #2 in BM25 => should tie or beat `c`.
        assert!(top_id == "a" || top_id == "c");
    }
}
