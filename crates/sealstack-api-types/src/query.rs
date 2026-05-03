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
///
/// Field order and names mirror `sealstack_engine::api::SearchResponse`
/// exactly so the gateway can pass the engine's response through unchanged.
/// SDK contract spec §7 documents this as the canonical wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryResponse {
    /// Receipt ID; resolves via `GET /v1/receipts/{id}`.
    pub receipt_id: String,
    /// Ranked hits.
    pub results: Vec<QueryHit>,
}

/// One ranked hit in a [`QueryResponse`].
///
/// Field shape mirrors `sealstack_engine::api::SearchHit` exactly so the
/// gateway can pass engine hits through without per-field conversion.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryHit {
    /// Primary key of the matched record.
    pub id: String,
    /// Combined hybrid score.
    pub score: f32,
    /// Snippet of text likely to have matched.
    pub excerpt: String,
    /// The full record as a JSON object.
    pub record: Value,
}
