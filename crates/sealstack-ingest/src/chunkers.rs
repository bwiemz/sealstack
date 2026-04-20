//! Chunking strategies: fixed, recursive, semantic, layout-aware, AST-aware.
//!
//! This module is a thin shim. The real chunking happens in
//! [`sealstack_engine::ingest::chunk_body`] so it can run against the engine's
//! configured `ChunkingStrategy` from each schema. What's here is a tiny
//! character-based chunker kept for ad-hoc use from the CLI and tests.

/// Split `text` into fixed-size character chunks. Returns the input as a
/// single chunk when `size == 0`.
#[must_use]
pub fn fixed(text: &str, size: usize) -> Vec<String> {
    if size == 0 {
        return vec![text.to_owned()];
    }
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size)
        .map(|c| c.iter().collect::<String>())
        .collect()
}
