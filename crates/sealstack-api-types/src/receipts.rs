//! Wire types for `GET /v1/receipts/{id}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Public projection of `sealstack_engine::Receipt` for the REST surface.
/// Intentionally narrower than the engine's internal Receipt: drops
/// `qualified_schema`, `tool`, `arguments`, `policies_applied`, and
/// `timings_ms`; renames `created_at` → `issued_at`; and surfaces `tenant`
/// at the top level (engine has it nested in `caller`). The gateway maps
/// fields at the response boundary.
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
    // `time::OffsetDateTime` doesn't implement `JsonSchema`; tell schemars
    // to render it as the same `String` shape that the rfc3339 serde adapter
    // emits. Both attributes are load-bearing — deleting either silently
    // breaks JSON Schema generation or wire serialization.
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
