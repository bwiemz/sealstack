//! Wire types for `GET /v1/receipts/{id}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Wire-shape mirror of the engine's `Receipt`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReceiptWire {
    /// Receipt ID (ULID).
    pub id: String,
    /// Caller identity at query time.
    pub caller_id: String,
    /// Tenant the query ran against.
    pub tenant: String,
    /// Source records that contributed to the answer.
    pub sources: Vec<ReceiptSource>,
    /// Issue timestamp (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    pub issued_at: OffsetDateTime,
}

/// One contributing source row in a [`ReceiptWire`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReceiptSource {
    /// Chunk ID this source resolves to.
    pub chunk_id: String,
    /// Source URI for the human reader.
    pub source_uri: String,
    /// Hybrid score for this contribution.
    pub score: f32,
}
