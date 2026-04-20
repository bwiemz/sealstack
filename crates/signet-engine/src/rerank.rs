//! Reranker abstraction.
//!
//! The retrieval pipeline produces a candidate set of chunks; the reranker
//! refines ordering using a stronger (and more expensive) signal — typically a
//! cross-encoder like `bge-reranker-v2` or `voyage-rerank-2`.
//!
//! # v0.1 implementations
//!
//! * [`IdentityReranker`] — returns candidates unchanged. Default when no
//!   reranker is configured for a schema.
//! * [`HttpReranker`] — calls an OpenAI-compatible `/v1/rerank` endpoint.
//!   Feature-gated on `reranker`.

use async_trait::async_trait;

use crate::api::EngineError;

/// One candidate entering the reranker.
#[derive(Debug, Clone)]
pub struct RerankCandidate {
    /// Primary key of the source record.
    pub id: String,
    /// Text snippet to score against the query.
    pub text: String,
    /// Pre-rerank score from the retrieval stage.
    pub prior_score: f32,
}

/// One candidate leaving the reranker.
#[derive(Debug, Clone)]
pub struct RerankResult {
    /// Same `id` as the corresponding [`RerankCandidate`].
    pub id: String,
    /// Final score in `[0.0, 1.0]`.
    pub score: f32,
}

/// Reranker trait.
#[async_trait]
pub trait Reranker: Send + Sync + 'static {
    /// Short identifier (matches a CSL `context { reranker = "<name>" }` value).
    fn name(&self) -> &str;

    /// Rerank `candidates` against `query`, returning new scores.
    ///
    /// The output length equals the input length. Caller sorts by score.
    async fn rerank(
        &self,
        query: &str,
        candidates: &[RerankCandidate],
    ) -> Result<Vec<RerankResult>, EngineError>;
}

/// Pass-through reranker: returns the prior score unchanged.
pub struct IdentityReranker;

#[async_trait]
impl Reranker for IdentityReranker {
    fn name(&self) -> &str {
        "identity"
    }

    async fn rerank(
        &self,
        _query: &str,
        candidates: &[RerankCandidate],
    ) -> Result<Vec<RerankResult>, EngineError> {
        Ok(candidates
            .iter()
            .map(|c| RerankResult {
                id: c.id.clone(),
                score: c.prior_score,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// HTTP reranker (OpenAI-compatible rerank API).
// ---------------------------------------------------------------------------

/// Reranker that calls an HTTP endpoint following the OpenAI `/v1/rerank` shape.
///
/// Compatible out of the box with Cohere Rerank, Voyage Rerank, Jina Rerank, and
/// self-hosted BGE rerankers exposed via `text-embeddings-inference`.
#[cfg(feature = "reranker")]
pub struct HttpReranker {
    /// Endpoint base URL (e.g. `https://api.voyageai.com/v1/rerank`).
    pub endpoint: String,
    /// API key for the `Authorization` header.
    pub api_key: Option<String>,
    /// Logical model name passed in the request body.
    pub model: String,
    /// Lazily-initialized HTTP client.
    pub client: reqwest::Client,
}

#[cfg(feature = "reranker")]
#[async_trait]
impl Reranker for HttpReranker {
    fn name(&self) -> &str {
        &self.model
    }

    async fn rerank(
        &self,
        query: &str,
        candidates: &[RerankCandidate],
    ) -> Result<Vec<RerankResult>, EngineError> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            model: &'a str,
            query: &'a str,
            documents: Vec<&'a str>,
        }
        #[derive(serde::Deserialize)]
        struct RespRow {
            index: usize,
            relevance_score: f32,
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            data: Vec<RespRow>,
        }

        let docs: Vec<&str> = candidates.iter().map(|c| c.text.as_str()).collect();
        let mut req = self
            .client
            .post(&self.endpoint)
            .json(&Req {
                model: &self.model,
                query,
                documents: docs,
            });
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp: Resp = req
            .send()
            .await
            .map_err(EngineError::backend)?
            .error_for_status()
            .map_err(EngineError::backend)?
            .json()
            .await
            .map_err(EngineError::backend)?;

        let mut out = vec![
            RerankResult {
                id: String::new(),
                score: 0.0,
            };
            candidates.len()
        ];
        for row in resp.data {
            if let Some(c) = candidates.get(row.index) {
                out[row.index] = RerankResult {
                    id: c.id.clone(),
                    score: row.relevance_score,
                };
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identity_preserves_order_and_scores() {
        let r = IdentityReranker;
        let cs = vec![
            RerankCandidate {
                id: "a".into(),
                text: "x".into(),
                prior_score: 0.9,
            },
            RerankCandidate {
                id: "b".into(),
                text: "y".into(),
                prior_score: 0.1,
            },
        ];
        let out = r.rerank("q", &cs).await.unwrap();
        assert_eq!(out.len(), 2);
        assert!((out[0].score - 0.9).abs() < 1e-6);
        assert!((out[1].score - 0.1).abs() < 1e-6);
    }
}
