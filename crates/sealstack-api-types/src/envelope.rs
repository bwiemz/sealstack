//! Wire envelope and error taxonomy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Discriminated-union response envelope. On success, `data` is `T` and
/// `error` is `null`. On failure, `data` is `null` and `error` carries
/// a code from [`ErrorCode`] and a human-readable message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Envelope<T> {
    /// Success payload, or `null` on failure.
    pub data: Option<T>,
    /// Error payload, or `null` on success.
    pub error: Option<ErrorDetail>,
}

/// Error detail returned in [`Envelope::error`].
///
/// v0.3 wire shape is `{ code, message }` only. SDK contract spec §8 lists
/// per-class typed attributes (`PolicyDeniedError.predicate`,
/// `InvalidArgumentError.field`/`reason`, etc.) — those are populated by
/// the SDK from response headers (`X-Request-Id` → `BackendError.request_id`;
/// `Retry-After` → `RateLimitedError.retry_after`) or are deferred to a
/// v0.4 envelope addition that adds an `Option<Value> details` field. The
/// `message` field is human-readable and SDKs must not parse it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ErrorDetail {
    /// Closed-set error code; see [`ErrorCode`].
    pub code: ErrorCode,
    /// Human-readable message. Not part of the contract; do not parse.
    pub message: String,
}

/// Closed-set error code emitted by the gateway. SDKs map each variant
/// to a typed exception class (see SDK contract spec §8).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Resource does not exist.
    NotFound,
    /// Schema does not exist; subclass of `NotFound` in SDKs.
    UnknownSchema,
    /// Authentication required or invalid.
    Unauthorized,
    /// Policy denied the operation; carries predicate name in message.
    PolicyDenied,
    /// Request shape was malformed; carries field name in message.
    InvalidArgument,
    /// Rate limit exceeded; reserved for v0.4 (gateway does not yet emit).
    RateLimited,
    /// Generic server error; carries `request_id` for diagnostics.
    Backend,
}
