//! OpenAI embeddings client — also compatible with OpenAI-API-shaped services
//! (Together AI, Groq, vLLM, LiteLLM proxy, text-embeddings-inference).
//!
//! Endpoint: `POST https://api.openai.com/v1/embeddings`.
//! Docs: <https://platform.openai.com/docs/api-reference/embeddings>
//!
//! Default model is `text-embedding-3-small` (1536 dims). The OpenAI v3
//! endpoint supports a `dimensions` parameter to truncate output vectors for
//! storage savings; we expose it via [`OpenAIEmbedder::with_dimensions`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Embedder, SealStackError, SealStackResult};

const DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1/embeddings";

/// The OpenAI embedder.
pub struct OpenAIEmbedder {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    dims: usize,
    /// If set, the v3 `dimensions` parameter to request a shorter vector.
    request_dims: Option<usize>,
}

impl OpenAIEmbedder {
    /// Construct with `text-embedding-3-small` and default endpoint.
    ///
    /// Reads the API key from `OPENAI_API_KEY` if `api_key` is empty.
    pub fn new(api_key: impl Into<String>) -> SealStackResult<Self> {
        Self::with_model(api_key, "text-embedding-3-small")
    }

    /// Construct with a specific model.
    pub fn with_model(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> SealStackResult<Self> {
        let model = model.into();
        let dims = dims_for_model(&model).ok_or_else(|| {
            SealStackError::Config(format!(
                "unknown openai embedding model `{model}`; set dims via `with_dims`"
            ))
        })?;
        let api_key = {
            let provided = api_key.into();
            if provided.is_empty() {
                std::env::var("OPENAI_API_KEY")
                    .map_err(|_| SealStackError::Config("missing OPENAI_API_KEY".into()))?
            } else {
                provided
            }
        };
        Ok(Self {
            client: reqwest::Client::new(),
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            api_key,
            model,
            dims,
            request_dims: None,
        })
    }

    /// Point at a different endpoint (proxy, Azure, self-hosted TEI, etc.).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Override the reported dims (for unknown / self-hosted models).
    #[must_use]
    pub fn with_dims(mut self, dims: usize) -> Self {
        self.dims = dims;
        self
    }

    /// Request the OpenAI v3 `dimensions` parameter for output truncation.
    ///
    /// Only the v3 family (`text-embedding-3-*`) supports this. Must be less
    /// than the model's native dimension count; the server normalizes the
    /// output so cosine similarity is preserved.
    #[must_use]
    pub fn with_dimensions(mut self, dims: usize) -> Self {
        self.request_dims = Some(dims);
        self.dims = dims;
        self
    }
}

/// Known OpenAI model → native dimensions.
fn dims_for_model(model: &str) -> Option<usize> {
    match model {
        "text-embedding-3-small" => Some(1536),
        "text-embedding-3-large" => Some(3072),
        "text-embedding-ada-002" => Some(1536),
        _ => None,
    }
}

#[derive(Serialize)]
struct EmbedReq<'a> {
    model: &'a str,
    input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding_format: Option<&'static str>,
}

#[derive(Deserialize)]
struct EmbedResp {
    data: Vec<EmbedRow>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct EmbedRow {
    embedding: Vec<f32>,
    #[serde(default)]
    index: usize,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Usage {
    total_tokens: u64,
}

#[async_trait]
impl Embedder for OpenAIEmbedder {
    fn name(&self) -> &str {
        &self.model
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn max_batch(&self) -> usize {
        2048
    }

    async fn embed(&self, texts: Vec<String>) -> SealStackResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if texts.len() > self.max_batch() {
            return Err(SealStackError::Validation(format!(
                "openai batch limit is {}, got {}",
                self.max_batch(),
                texts.len()
            )));
        }

        let payload = EmbedReq {
            model: &self.model,
            input: texts.clone(),
            dimensions: self.request_dims,
            encoding_format: Some("float"),
        };

        let resp = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(SealStackError::backend)?;

        let status = resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SealStackError::RateLimited);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SealStackError::Backend(format!("openai {status}: {body}")));
        }

        let parsed: EmbedResp = resp.json().await.map_err(SealStackError::backend)?;
        let mut rows = parsed.data;
        rows.sort_by_key(|r| r.index);

        if rows.len() != texts.len() {
            return Err(SealStackError::Backend(format!(
                "openai returned {} rows for {} inputs",
                rows.len(),
                texts.len()
            )));
        }
        for (i, row) in rows.iter().enumerate() {
            if row.embedding.len() != self.dims {
                return Err(SealStackError::Backend(format!(
                    "openai row {i}: dims {} != expected {}",
                    row.embedding.len(),
                    self.dims
                )));
            }
        }

        if let Some(usage) = parsed.usage {
            tracing::debug!(tokens = usage.total_tokens, "openai embedded batch");
        }
        Ok(rows.into_iter().map(|r| r.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_resolve() {
        assert_eq!(dims_for_model("text-embedding-3-small"), Some(1536));
        assert_eq!(dims_for_model("text-embedding-3-large"), Some(3072));
        assert_eq!(dims_for_model("text-embedding-ada-002"), Some(1536));
    }

    #[test]
    fn unknown_model_rejected() {
        match OpenAIEmbedder::with_model("k", "nonsense") {
            Err(SealStackError::Config(_)) => {}
            Err(other) => panic!("expected Config error, got: {other}"),
            Ok(_) => panic!("expected error for unknown model"),
        }
    }

    #[test]
    fn with_dimensions_truncates() {
        let e = OpenAIEmbedder::with_model("k", "text-embedding-3-large")
            .unwrap()
            .with_dimensions(256);
        assert_eq!(e.dims(), 256);
    }
}
