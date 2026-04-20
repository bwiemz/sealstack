//! Vector-store abstraction.
//!
//! Implementations:
//!
//! * [`memory::InMemoryStore`] — in-process, backed by `DashMap`. Linear cosine
//!   search. Good for tests, local dev, and small-scale single-node deployments.
//! * [`qdrant::QdrantStore`] — Qdrant HTTP/gRPC client (`qdrant-client`).
//!   Feature-gated on `qdrant`.
//!
//! Adding a new backend means implementing [`VectorStore`] and registering the
//! constructor in the engine's boot code.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use sealstack_common::{SealStackError, SealStackResult};

#[cfg(feature = "memory")]
pub mod memory;
#[cfg(feature = "qdrant")]
pub mod qdrant;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// One chunk of ingestion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    /// Stable chunk id.
    pub id: ulid::Ulid,
    /// Text content (the excerpt the LLM will see).
    pub content: String,
    /// Embedding vector. Length equals the collection's configured dims.
    pub embedding: Vec<f32>,
    /// Payload metadata — arbitrary JSON object. Fields used for filtering
    /// should be flat scalars for best backend performance.
    pub metadata: serde_json::Map<String, Value>,
}

/// One hit from a `search` call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    /// Chunk id.
    pub id: ulid::Ulid,
    /// Similarity score. For cosine distance backends this is `1 - distance`
    /// clamped to `[0, 1]`; raw scores are passed through otherwise.
    pub score: f32,
    /// Excerpt text (mirrors `Chunk::content`).
    pub content: String,
    /// Payload metadata.
    pub metadata: serde_json::Map<String, Value>,
}

/// Collection configuration handed to `ensure_collection`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionSpec {
    /// Collection name (e.g. `"customer_v2"`).
    pub name: String,
    /// Embedding dimensions.
    pub dims: usize,
    /// Distance metric.
    #[serde(default)]
    pub distance: Distance,
}

/// Distance metric for similarity search.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distance {
    /// Cosine similarity (default, normalized).
    #[default]
    Cosine,
    /// Dot product.
    Dot,
    /// Euclidean L2.
    Euclidean,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// The vector-store trait.
///
/// Engine code only ever sees `Arc<dyn VectorStore>`. Implementations must be
/// safe to call concurrently from many tasks.
#[async_trait]
pub trait VectorStore: Send + Sync + 'static {
    /// Short identifier for logs / diagnostics (e.g., `"memory"`, `"qdrant"`).
    fn kind(&self) -> &'static str;

    /// Ensure a collection exists with the given dimensions.
    ///
    /// Idempotent. Distance defaults to [`Distance::Cosine`].
    async fn ensure_collection(&self, name: &str, dims: usize) -> SealStackResult<()>;

    /// Ensure a collection exists with explicit settings.
    ///
    /// Default impl calls [`Self::ensure_collection`] — backends that need
    /// richer config should override.
    async fn ensure_collection_spec(&self, spec: &CollectionSpec) -> SealStackResult<()> {
        self.ensure_collection(&spec.name, spec.dims).await
    }

    /// Upsert chunks into a collection.
    async fn upsert(&self, collection: &str, chunks: Vec<Chunk>) -> SealStackResult<()>;

    /// Dense nearest-neighbor search.
    ///
    /// `filter` is an optional JSON object. Backends that support native
    /// filtering should translate it to their query language; others may
    /// post-filter in memory.
    async fn search(
        &self,
        collection: &str,
        query_vec: Vec<f32>,
        top_k: usize,
        filter: Option<Value>,
    ) -> SealStackResult<Vec<SearchResult>>;

    /// Delete chunks by id.
    async fn delete(&self, collection: &str, ids: Vec<ulid::Ulid>) -> SealStackResult<()>;

    /// Count chunks in a collection.
    async fn count(&self, collection: &str) -> SealStackResult<u64>;

    /// Remove an entire collection.
    async fn drop_collection(&self, name: &str) -> SealStackResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_default_is_cosine() {
        let d = Distance::default();
        assert!(matches!(d, Distance::Cosine));
    }
}
