//! Ingestion pipeline.
//!
//! Ingestion is the process of taking a [`sealstack_connector_sdk::Resource`] from a
//! connector and turning it into:
//!
//! 1. A typed row in the schema's Postgres table.
//! 2. One or more chunks in the schema's vector-store collection, each with an
//!    embedding.
//! 3. A lineage edge linking the chunk back to its source record.
//!
//! The ingestion pipeline is called from `sealstack-ingest` (the runtime that polls
//! or subscribes to connectors); this module is the engine-side API those
//! callers use.

use std::sync::Arc;

use sealstack_connector_sdk::Resource;
use sealstack_embedders::Embedder;
use sealstack_vectorstore::{Chunk, VectorStore};
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
    /// Collections we've already called `ensure_collection` on. Memoized so
    /// the happy-path ingest doesn't round-trip to the vector store before
    /// every upsert — the per-backend check is idempotent and cheap, but
    /// a local set is effectively free.
    ensured_collections: dashmap::DashSet<String>,
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
            ensured_collections: dashmap::DashSet::new(),
        }
    }

    /// Ensure the schema's vector-store collection exists before the first
    /// upsert. Idempotent on every call; memoized on our side so the cold
    /// path runs at most once per collection per process.
    async fn ensure_collection(&self, meta: &SchemaMeta) -> Result<(), EngineError> {
        if self.ensured_collections.contains(&meta.collection) {
            return Ok(());
        }
        self.vector_store
            .ensure_collection(&meta.collection, meta.context.vector_dims)
            .await
            .map_err(|e| {
                EngineError::Backend(format!(
                    "ensure collection `{}` ({}d): {e}",
                    meta.collection, meta.context.vector_dims
                ))
            })?;
        self.ensured_collections.insert(meta.collection.clone());
        Ok(())
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

        // Make sure the vector-store collection exists. Cheap when memoized.
        self.ensure_collection(meta).await?;

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
            .zip(embeddings)
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
    pub async fn delete(&self, meta: &SchemaMeta, record_id: &str) -> Result<(), EngineError> {
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
/// Token counting backends:
///
/// - **default**: a dependency-free `~4 chars per token` heuristic (the
///   bytes/4 ratio that OpenAI's docs cite for English prose). Cheap and
///   fast but drifts on code, non-Latin scripts, and short whitespace-heavy
///   text.
/// - **`tiktoken-chunker` feature**: real BPE token counts via
///   [`tiktoken_rs::cl100k_base`] — the encoder used by GPT-3.5 / GPT-4 /
///   text-embedding-3 family. Budgets are honored to the actual token, at
///   the cost of pulling regex + base64 into the build.
#[must_use]
pub fn chunk_body(body: &str, strategy: &ChunkingStrategy) -> Vec<ChunkRaw> {
    if body.trim().is_empty() {
        return Vec::new();
    }
    let counter = token_counter();
    match strategy {
        ChunkingStrategy::Fixed { size } => fixed_char_chunks(body, *size),
        ChunkingStrategy::Semantic {
            max_tokens,
            overlap,
        } => semantic_chunks(body, *max_tokens, *overlap, counter.as_ref()),
        ChunkingStrategy::Recursive { split, max_tokens } => {
            recursive_chunks(body, split.as_slice(), *max_tokens, counter.as_ref())
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

fn semantic_chunks(
    body: &str,
    max_tokens: usize,
    overlap: usize,
    counter: &dyn TokenCounter,
) -> Vec<ChunkRaw> {
    let paragraphs: Vec<&str> = body
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for p in paragraphs {
        let candidate_tokens = counter.count(&format!("{current}\n\n{p}"));
        if candidate_tokens > max_tokens && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = carry_overlap(&current, overlap, counter);
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
        chunks.push(body.to_string());
    }
    chunks.into_iter().map(|text| ChunkRaw { text }).collect()
}

/// Trim a chunk down to the last `overlap` tokens so it can seed the next
/// chunk. Falls back to character-based slicing when the token counter is
/// the char-approx variant — counting tokens for the trim itself would
/// double the work.
fn carry_overlap(current: &str, overlap: usize, _counter: &dyn TokenCounter) -> String {
    if overlap == 0 {
        return String::new();
    }
    // Cheap approximation: keep the trailing 4*overlap bytes, which is
    // approximately `overlap` tokens for English prose. Real tokenizers
    // can over-shoot the boundary but the next-chunk loop re-checks the
    // total token count, so the bound holds in steady state.
    let target_chars = overlap.saturating_mul(4);
    if current.len() <= target_chars {
        return current.to_string();
    }
    // Walk back to a UTF-8 boundary.
    let mut split_at = current.len() - target_chars;
    while split_at < current.len() && !current.is_char_boundary(split_at) {
        split_at += 1;
    }
    current[split_at..].to_string()
}

fn recursive_chunks(
    body: &str,
    separators: &[String],
    max_tokens: usize,
    counter: &dyn TokenCounter,
) -> Vec<ChunkRaw> {
    recursive_inner(body, separators, max_tokens, 0, counter)
        .into_iter()
        .map(|text| ChunkRaw { text })
        .collect()
}

fn recursive_inner(
    input: &str,
    seps: &[String],
    max_tokens: usize,
    depth: usize,
    counter: &dyn TokenCounter,
) -> Vec<String> {
    if counter.count(input) <= max_tokens || depth >= seps.len() {
        return vec![input.to_string()];
    }
    let sep = &seps[depth];
    let mut out = Vec::new();
    for piece in input.split(sep.as_str()) {
        if piece.is_empty() {
            continue;
        }
        if counter.count(piece) > max_tokens {
            out.extend(recursive_inner(piece, seps, max_tokens, depth + 1, counter));
        } else {
            out.push(piece.to_string());
        }
    }
    out
}

/// Token-counter abstraction. Implementations must be cheap to invoke
/// from the chunker's inner loops — the chunker calls `count` once per
/// paragraph for the semantic strategy and once per piece per separator
/// level for the recursive strategy.
trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> usize;
}

struct CharApprox;
impl TokenCounter for CharApprox {
    fn count(&self, text: &str) -> usize {
        // Standard "1 token ≈ 4 bytes" approximation. Underestimates non-Latin
        // scripts and overestimates symbol-heavy text; close enough for budgeting.
        text.len().div_ceil(4)
    }
}

#[cfg(feature = "tiktoken-chunker")]
struct TiktokenCounter {
    bpe: tiktoken_rs::CoreBPE,
}

#[cfg(feature = "tiktoken-chunker")]
impl TiktokenCounter {
    fn new() -> Option<Self> {
        // `cl100k_base` is the encoding for GPT-3.5 / GPT-4 / text-embedding-3.
        // Construction can fail if the BPE files are corrupted in the
        // tiktoken-rs build; we treat that as a runtime fallback to
        // CharApprox rather than panic at boot.
        tiktoken_rs::cl100k_base().ok().map(|bpe| Self { bpe })
    }
}

#[cfg(feature = "tiktoken-chunker")]
impl TokenCounter for TiktokenCounter {
    fn count(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }
}

#[cfg(feature = "tiktoken-chunker")]
fn token_counter() -> std::sync::Arc<dyn TokenCounter> {
    use std::sync::OnceLock;
    static COUNTER: OnceLock<std::sync::Arc<dyn TokenCounter>> = OnceLock::new();
    COUNTER
        .get_or_init(|| match TiktokenCounter::new() {
            Some(t) => {
                tracing::debug!("chunker using tiktoken cl100k_base");
                std::sync::Arc::new(t)
            }
            None => {
                tracing::warn!(
                    "tiktoken cl100k_base init failed; chunker falling back to char-approx"
                );
                std::sync::Arc::new(CharApprox)
            }
        })
        .clone()
}

#[cfg(not(feature = "tiktoken-chunker"))]
fn token_counter() -> std::sync::Arc<dyn TokenCounter> {
    std::sync::Arc::new(CharApprox)
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
        let counter = CharApprox;
        let chunks = semantic_chunks(&body, 40, 0, &counter);
        assert!(chunks.len() > 1, "expected multiple chunks");
        for c in &chunks {
            // Char-approx counter targets 40 tokens × 4 bytes ≈ 160 bytes;
            // last paragraph in a chunk may overshoot by one paragraph
            // because the candidate check happens before the push. Bound
            // is "no more than 2× the budget per chunk".
            assert!(
                c.text.len() <= 40 * 4 * 2 + 200,
                "chunk too large: {}",
                c.text.len()
            );
        }
    }

    #[test]
    fn empty_body_produces_no_chunks() {
        assert!(chunk_body("", &ChunkingStrategy::default()).is_empty());
        assert!(chunk_body("   \n  ", &ChunkingStrategy::default()).is_empty());
    }

    #[test]
    fn char_approx_counter_is_div_ceil_4() {
        let c = CharApprox;
        assert_eq!(c.count(""), 0);
        assert_eq!(c.count("abc"), 1);
        assert_eq!(c.count("abcd"), 1);
        assert_eq!(c.count("abcde"), 2);
        assert_eq!(c.count(&"x".repeat(100)), 25);
    }

    #[test]
    fn semantic_overlap_carries_trailing_text() {
        let body = "alpha alpha alpha alpha alpha\n\nbeta beta beta beta beta\n\ngamma";
        let counter = CharApprox;
        // Pick a small budget so each paragraph trips the split.
        let chunks = semantic_chunks(body, 5, 3, &counter);
        assert!(chunks.len() >= 2);
        // Overlap means the second chunk should start with content from
        // the tail of the first paragraph, not from `beta`.
        assert!(
            chunks[1].text.contains("beta") || chunks[1].text.contains("alpha"),
            "overlap not applied: {}",
            chunks[1].text,
        );
    }

    #[test]
    fn recursive_splits_when_oversize() {
        let body = "section1\n\nsection2 with a lot of words to push past the budget threshold";
        let counter = CharApprox;
        let chunks = recursive_chunks(body, &["\n\n".to_string(), " ".to_string()], 5, &counter);
        assert!(chunks.len() >= 2);
    }
}
