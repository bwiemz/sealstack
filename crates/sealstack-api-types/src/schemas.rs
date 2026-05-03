//! Wire types for `/v1/schemas` and `/v1/schemas/{q}/ddl`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request body for `POST /v1/schemas`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterSchemaRequest {
    /// Schema metadata as emitted by `sealstack_csl::codegen`. Free-shaped
    /// here to avoid coupling api-types to the CSL crate; gateway parses
    /// into the typed `sealstack_engine::SchemaMeta` internally.
    pub meta: Value,
}

/// Response data for `POST /v1/schemas`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterSchemaResponse {
    /// Qualified schema name (`<namespace>.<name>`).
    pub qualified: String,
}

/// Request body for `POST /v1/schemas/{qualified}/ddl`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyDdlRequest {
    /// Postgres DDL text (CREATE TABLE / CREATE INDEX / ...).
    pub ddl: String,
}

/// Response data for `POST /v1/schemas/{qualified}/ddl`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyDdlResponse {
    /// Number of statements applied.
    pub applied: u32,
}

/// Response data for `GET /v1/schemas` and `GET /v1/schemas/{q}`.
///
/// Wire-shape mirror of `sealstack_engine::SchemaMeta`. The duplication
/// keeps `sealstack-api-types` free of engine deps; the gateway converts
/// at the response boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SchemaMetaWire {
    /// Namespace, e.g. `"examples"`.
    pub namespace: String,
    /// Schema name, e.g. `"Doc"`.
    pub name: String,
    /// Schema-version integer.
    pub version: u32,
    /// Field name used as primary key.
    pub primary_key: String,
    /// Postgres table name.
    pub table: String,
    /// Vector store collection name.
    pub collection: String,
    /// Hybrid score blend factor.
    pub hybrid_alpha: f32,
}

/// Wrapper for `GET /v1/schemas` response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSchemasResponse {
    /// Registered schemas.
    pub schemas: Vec<SchemaMetaWire>,
}
