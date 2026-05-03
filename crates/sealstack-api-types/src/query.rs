//! Wire types for `POST /v1/query`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request body for `POST /v1/query`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryRequest {
    /// Qualified schema name, e.g. `"examples.Doc"`.
    pub schema: String,
    /// Query string (natural-language or keywords).
    pub query: String,
    /// Cap on results; `None` defaults server-side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    /// Filter expression; structure depends on schema's facet declarations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Value>,
}

/// Response data for `POST /v1/query`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryResponse {
    /// Ranked hits.
    pub hits: Vec<QueryHit>,
    /// Receipt ID; resolves via `GET /v1/receipts/{id}`.
    pub receipt_id: String,
}

/// One ranked hit in a [`QueryResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryHit {
    /// Resource identifier.
    pub id: String,
    /// Display title or subject line.
    pub title: Option<String>,
    /// Snippet to render in UI.
    pub snippet: Option<String>,
    /// Combined hybrid score.
    pub score: f32,
}
