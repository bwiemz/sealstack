//! Wire types for `/healthz` and `/readyz`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Response data for `GET /healthz` and `GET /readyz`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatusKind {
    /// Service is fully ready.
    Ok,
    /// Service is starting; not ready to take traffic.
    Starting,
}

/// Body shape for health endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HealthStatus {
    /// Status discriminator.
    pub status: HealthStatusKind,
}
