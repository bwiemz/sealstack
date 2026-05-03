//! Wire types for `/v1/connectors`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request body for `POST /v1/connectors`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterConnectorRequest {
    /// Connector kind (`"local-files"`, `"github"`, `"slack"`, `"google-drive"`).
    pub kind: String,
    /// Qualified schema name this connector binds to.
    pub schema: String,
    /// Free-shaped connector-specific config (root path, OAuth token, etc.).
    pub config: Value,
}

/// Response data for `POST /v1/connectors`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterConnectorResponse {
    /// Connector binding ID (`<kind>/<qualified>`).
    pub id: String,
}

/// Wire-shape mirror of `sealstack_ingest::ConnectorBindingInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectorBindingWire {
    /// Binding ID.
    pub id: String,
    /// Connector kind.
    pub kind: String,
    /// Qualified schema name.
    pub schema: String,
    /// Whether the binding is enabled.
    pub enabled: bool,
}

/// Response data for `GET /v1/connectors`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListConnectorsResponse {
    /// Registered connector bindings.
    pub connectors: Vec<ConnectorBindingWire>,
}

/// Response data for `POST /v1/connectors/{id}/sync`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SyncConnectorResponse {
    /// Job identifier for the sync run.
    pub job_id: String,
}
