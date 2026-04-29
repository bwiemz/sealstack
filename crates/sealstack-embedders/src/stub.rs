//! Deterministic stub embedder.
//!
//! Hashes input text with BLAKE3 and expands the hash into a pseudo-random but
//! **deterministic** vector. Same input → same vector, always. Different inputs
//! with identical prefixes produce different vectors.
//!
//! This backend is semantically useless — it encodes no meaning — but it is
//! enough to exercise the retrieval pipeline end-to-end in tests and local
//! development. Queries matching an ingested document *exactly* will get a
//! cosine of 1.0; near-duplicate queries land near-randomly.

use async_trait::async_trait;

use crate::{Embedder, SealStackResult};

/// The stub embedder.
pub struct StubEmbedder {
    dims: usize,
}

impl StubEmbedder {
    /// Construct a stub embedder producing `dims`-dimensional vectors.
    #[must_use]
    pub fn new(dims: usize) -> Self {
        assert!(dims > 0, "dims must be > 0");
        Self { dims }
    }
}

#[async_trait]
impl Embedder for StubEmbedder {
    fn name(&self) -> &str {
        "stub"
    }

    fn dims(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: Vec<String>) -> SealStackResult<Vec<Vec<f32>>> {
        Ok(texts
            .into_iter()
            .map(|t| hash_to_vec(&t, self.dims))
            .collect())
    }
}

/// Hash `text` with BLAKE3 and expand the digest into `dims` f32s in `[-1, 1]`.
///
/// We walk the hash output in 4-byte chunks, reinterpreting each as a `u32`,
/// normalizing to `[0, 1]`, and centering to `[-1, 1]`. The blake3 hasher is
/// extendable-output, so we can produce as many bytes as needed for any `dims`.
fn hash_to_vec(text: &str, dims: usize) -> Vec<f32> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(text.as_bytes());
    let mut reader = hasher.finalize_xof();

    let mut out = Vec::with_capacity(dims);
    let mut buf = [0u8; 4];
    for _ in 0..dims {
        reader.fill(&mut buf);
        let n = u32::from_le_bytes(buf);
        // Normalize to [-1, 1].
        let v = (f64::from(n) / f64::from(u32::MAX)) * 2.0 - 1.0;
        out.push(v as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_input_same_vector() {
        let e = StubEmbedder::new(8);
        let a = e.embed(vec!["hello".into()]).await.unwrap();
        let b = e.embed(vec!["hello".into()]).await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn different_inputs_different_vectors() {
        let e = StubEmbedder::new(8);
        let a = e.embed(vec!["hello".into()]).await.unwrap();
        let b = e.embed(vec!["world".into()]).await.unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn dims_are_respected() {
        let e = StubEmbedder::new(32);
        let v = e.embed(vec!["x".into()]).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].len(), 32);
    }

    #[tokio::test]
    async fn values_are_in_range() {
        let e = StubEmbedder::new(64);
        let v = e.embed(vec!["range-check".into()]).await.unwrap();
        for x in &v[0] {
            assert!(*x >= -1.0 && *x <= 1.0, "out of range: {x}");
        }
    }

    #[tokio::test]
    async fn batch_preserves_order() {
        let e = StubEmbedder::new(8);
        let v = e
            .embed(vec!["a".into(), "b".into(), "c".into()])
            .await
            .unwrap();
        assert_eq!(v.len(), 3);
        // Each row should be different.
        assert_ne!(v[0], v[1]);
        assert_ne!(v[1], v[2]);
    }
}
