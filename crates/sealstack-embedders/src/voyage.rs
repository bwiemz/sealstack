//! Voyage AI embeddings client.
//!
//! Endpoint: `POST https://api.voyageai.com/v1/embeddings`.
//! Docs: <https://docs.voyageai.com/reference/embeddings-api>
//!
//! Default model is `voyage-3`, which produces 1024-dimensional vectors. The
//! constructor validates the model name against a small catalog so mismatches
//! surface as construction-time errors rather than as confusing 400s at
//! request time.
//!
//! # Rate limits
//!
//! Voyage enforces a tokens-per-minute ceiling that varies by tier. This client
//! does not rate-limit locally — the engine's embed budget should be set at
//! ingest time based on the customer's Voyage tier.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{SealStackError, SealStackResult, Embedder};

const DEFAULT_ENDPOINT: &str = "https://api.voyageai.com/v1/embeddings";

/// Input-type hint Voyage uses to tune encoding.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    /// General-purpose encoding (default).
    #[default]
    None,
    /// Document / passage being indexed.
    Document,
    /// Search query.
    Query,
}

impl InputType {
    fn as_api_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Document => Some("document"),
            Self::Query => Some("query"),
        }
    }
}

/// The Voyage embedder.
pub struct VoyageEmbedder {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    dims: usize,
    input_type: InputType,
}

impl VoyageEmbedder {
    /// Construct with default endpoint and the `voyage-3` model.
    ///
    /// Reads the API key from `VOYAGE_API_KEY` if `api_key` is empty.
    pub fn new(api_key: impl Into<String>) -> SealStackResult<Self> {
        Self::with_model(api_key, "voyage-3")
    }

    /// Construct with a specific model.
    pub fn with_model(api_key: impl Into<String>, model: impl Into<String>) -> SealStackResult<Self> {
        let model = model.into();
        let dims = dims_for_model(&model).ok_or_else(|| {
            SealStackError::Config(format!(
                "unknown voyage model `{model}`; add it to `dims_for_model`"
            ))
        })?;
        let api_key = {
            let provided = api_key.into();
            if provided.is_empty() {
                std::env::var("VOYAGE_API_KEY")
                    .map_err(|_| SealStackError::Config("missing VOYAGE_API_KEY".into()))?
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
            input_type: InputType::default(),
        })
    }

    /// Override the HTTP endpoint (useful for a proxy or mock server).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Hint the input type for Voyage's encoder.
    #[must_use]
    pub fn with_input_type(mut self, ty: InputType) -> Self {
        self.input_type = ty;
        self
    }

    /// Override the dims (useful for experimental models not in the built-in catalog).
    #[must_use]
    pub fn with_dims(mut self, dims: usize) -> Self {
        self.dims = dims;
        self
    }
}

/// Known Voyage model → dimensions.
fn dims_for_model(model: &str) -> Option<usize> {
    match model {
        "voyage-3" => Some(1024),
        "voyage-3-large" => Some(1024),
        "voyage-3-lite" => Some(512),
        "voyage-code-3" => Some(1024),
        "voyage-finance-2" => Some(1024),
        "voyage-law-2" => Some(1024),
        "voyage-multilingual-2" => Some(1024),
        // Older family kept for compatibility.
        "voyage-2" => Some(1024),
        "voyage-large-2" => Some(1536),
        _ => None,
    }
}

#[derive(Serialize)]
struct EmbedReq<'a> {
    model: &'a str,
    input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<&'static str>,
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
impl Embedder for VoyageEmbedder {
    fn name(&self) -> &str {
        &self.model
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn max_batch(&self) -> usize {
        128
    }

    async fn embed(&self, texts: Vec<String>) -> SealStackResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if texts.len() > self.max_batch() {
            return Err(SealStackError::Validation(format!(
                "voyage batch limit is {}, got {}",
                self.max_batch(),
                texts.len()
            )));
        }

        let payload = EmbedReq {
            model: &self.model,
            input: texts.clone(),
            input_type: self.input_type.as_api_str(),
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
            return Err(SealStackError::Backend(format!(
                "voyage {status}: {body}"
            )));
        }

        let parsed: EmbedResp = resp.json().await.map_err(SealStackError::backend)?;

        // Voyage returns rows with `index`; re-sort to match input order just
        // in case the server reorders on retry.
        let mut rows = parsed.data;
        rows.sort_by_key(|r| r.index);

        if rows.len() != texts.len() {
            return Err(SealStackError::Backend(format!(
                "voyage returned {} rows for {} inputs",
                rows.len(),
                texts.len()
            )));
        }
        for (i, row) in rows.iter().enumerate() {
            if row.embedding.len() != self.dims {
                return Err(SealStackError::Backend(format!(
                    "voyage row {i}: dims {} != expected {}",
                    row.embedding.len(),
                    self.dims
                )));
            }
        }

        if let Some(usage) = parsed.usage {
            tracing::debug!(tokens = usage.total_tokens, "voyage embedded batch");
        }
        Ok(rows.into_iter().map(|r| r.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_resolves_dims() {
        assert_eq!(dims_for_model("voyage-3"), Some(1024));
        assert_eq!(dims_for_model("voyage-3-lite"), Some(512));
        assert_eq!(dims_for_model("voyage-large-2"), Some(1536));
    }

    #[test]
    fn unknown_model_is_rejected_at_construction() {
        // Requires VOYAGE_API_KEY; here we verify the model path rejects first.
        match VoyageEmbedder::with_model("test_key", "voyage-nonexistent") {
            Err(SealStackError::Config(_)) => {}
            Err(other) => panic!("expected Config error, got: {other}"),
            Ok(_) => panic!("expected error for unknown model"),
        }
    }

    #[test]
    fn with_dims_overrides_catalog_value() {
        let e = VoyageEmbedder::with_model("test_key", "voyage-3")
            .unwrap()
            .with_dims(256);
        assert_eq!(e.dims(), 256);
    }
}
