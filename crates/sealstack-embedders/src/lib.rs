//! Embedder abstraction.
//!
//! Implementations:
//!
//! * [`stub::StubEmbedder`] — deterministic hash-to-vector mapping. Default for
//!   tests and local dev; identical inputs produce identical vectors without
//!   touching the network.
//! * [`voyage::VoyageEmbedder`] — Voyage AI `/v1/embeddings` client. Preferred
//!   for production retrieval quality (feature-gated on `voyage`).
//! * [`openai::OpenAIEmbedder`] — OpenAI `/v1/embeddings` client. Also compatible
//!   with OpenAI-API-shaped endpoints (Together, Groq, vLLM, LiteLLM).
//!   Feature-gated on `openai`.
//!
//! # Batching and limits
//!
//! Every implementation takes a full `Vec<String>` of inputs in one call. Vendor
//! APIs have per-request batch size limits (Voyage: 128, OpenAI: 2048 inputs);
//! callers that exceed these should split into chunks. The engine's
//! [`ingest`](../../sealstack_engine/ingest/index.html) path never batches beyond
//! 128 chunks per call.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

use async_trait::async_trait;

pub use sealstack_common::{SealStackError, SealStackResult};

#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "stub")]
pub mod stub;
#[cfg(feature = "voyage")]
pub mod voyage;

/// Embedder trait.
#[async_trait]
pub trait Embedder: Send + Sync + 'static {
    /// Short identifier (matches a CSL `context { embedder = "<n>" }` value).
    fn name(&self) -> &str;

    /// Output vector dimensionality.
    ///
    /// Must match the `vector_dims` in the schema's `context` block.
    fn dims(&self) -> usize;

    /// Maximum number of inputs per `embed` call. Defaults to `usize::MAX`.
    fn max_batch(&self) -> usize {
        usize::MAX
    }

    /// Produce an embedding for each input string.
    ///
    /// The returned vectors are in the same order as the inputs and each has
    /// length `self.dims()`.
    async fn embed(&self, texts: Vec<String>) -> SealStackResult<Vec<Vec<f32>>>;
}

#[cfg(test)]
mod tests {
    // Trait-only tests — backend tests live in each backend module.
}
