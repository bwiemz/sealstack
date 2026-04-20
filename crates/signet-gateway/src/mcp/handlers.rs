//! Default `ToolHandler` implementations bound at boot for every compiled schema.
//!
//! These handlers are generic by construction: each carries a qualified schema
//! name plus a [`HandlerKind`] discriminator, and dispatches to the
//! corresponding method on [`signet_engine::facade::EngineFacade`].
//!
//! The facade trait lives in `signet-engine` so the engine crate can provide the
//! single blanket impl (any `EngineHandle` becomes an `EngineFacade` via the
//! blanket in `signet_engine::facade`). The gateway does not know about the
//! concrete engine type — only the trait object.
//!
//! Handler responsibilities:
//! 1. Validate arguments against the CSL-generated JSON Schema.
//! 2. Convert the gateway's [`Caller`] into [`signet_engine::api::Caller`].
//! 3. Map engine-level errors onto the MCP-facing [`ToolError`] set.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use signet_engine::api::EngineError;
use signet_engine::facade::EngineFacade;

use super::registry::{ToolError, ToolHandler};
use super::types::{Caller, ToolDescriptor};

/// A concrete handler produced at boot from a CSL-generated tool descriptor.
pub struct GeneratedHandler {
    /// The pre-computed descriptor (from `signet_csl::codegen::mcp`).
    pub descriptor: ToolDescriptor,
    /// Classifies the handler shape so `invoke` can dispatch without re-parsing the name.
    pub kind: HandlerKind,
    /// Fully-qualified schema name the handler operates on.
    pub schema: String,
    /// Relation name (set only for `HandlerKind::ListRelation`).
    pub relation: Option<String>,
    /// Facet name (set only for `HandlerKind::Aggregate`).
    pub facet: Option<String>,
    /// The engine facade this handler delegates to.
    pub engine: Arc<dyn EngineFacade>,
}

/// Handler shapes we auto-generate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandlerKind {
    /// `search_<schema>` — hybrid vector + keyword search.
    Search,
    /// `get_<schema>` — single record fetch.
    Get,
    /// `list_<schema>` — paginated list with facets.
    List,
    /// `list_<schema>_<relation>` — relation walker.
    ListRelation,
    /// `aggregate_<schema>_<facet>` — facet histogram.
    Aggregate,
}

#[async_trait]
impl ToolHandler for GeneratedHandler {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    async fn invoke(&self, caller: &Caller, args: &Value) -> Result<Value, ToolError> {
        let engine_caller = caller_to_engine(caller);
        let result = match self.kind {
            HandlerKind::Search => {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError::InvalidArgs("missing `query`".into()))?;
                let top_k = args
                    .get("top_k")
                    .and_then(Value::as_u64)
                    .map_or(12, |n| n as usize);
                let empty = json!({});
                let filters = args.get("filters").unwrap_or(&empty);
                self.engine
                    .search(&engine_caller, &self.schema, query, top_k, filters)
                    .await
            }
            HandlerKind::Get => {
                let id = args
                    .get("id")
                    .ok_or_else(|| ToolError::InvalidArgs("missing `id`".into()))?;
                self.engine.get(&engine_caller, &self.schema, id).await
            }
            HandlerKind::List => {
                let empty = json!({});
                let filters = args.get("filters").unwrap_or(&empty);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map_or(20, |n| n as usize);
                let cursor = args.get("cursor").and_then(Value::as_str);
                let order_by = args.get("order_by").and_then(Value::as_str);
                let direction = args
                    .get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("desc");
                self.engine
                    .list(
                        &engine_caller,
                        &self.schema,
                        filters,
                        limit,
                        cursor,
                        order_by,
                        direction,
                    )
                    .await
            }
            HandlerKind::ListRelation => {
                let relation = self
                    .relation
                    .as_deref()
                    .ok_or_else(|| ToolError::Backend("relation handler missing rel name".into()))?;
                let parent_id = args
                    .get("parent_id")
                    .ok_or_else(|| ToolError::InvalidArgs("missing `parent_id`".into()))?;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map_or(20, |n| n as usize);
                let cursor = args.get("cursor").and_then(Value::as_str);
                self.engine
                    .list_relation(
                        &engine_caller,
                        &self.schema,
                        relation,
                        parent_id,
                        limit,
                        cursor,
                    )
                    .await
            }
            HandlerKind::Aggregate => {
                let facet = self
                    .facet
                    .as_deref()
                    .ok_or_else(|| ToolError::Backend("aggregate handler missing facet".into()))?;
                let empty = json!({});
                let filters = args.get("filters").unwrap_or(&empty);
                let buckets = args
                    .get("buckets")
                    .and_then(Value::as_u64)
                    .map_or(20, |n| n as usize);
                self.engine
                    .aggregate(&engine_caller, &self.schema, facet, filters, buckets)
                    .await
            }
        };
        result.map_err(engine_error_to_tool_error)
    }
}

// ---------------------------------------------------------------------------
// Conversions — kept private so the rest of the crate does not reach into
// signet-engine's type namespace.
// ---------------------------------------------------------------------------

fn caller_to_engine(c: &Caller) -> signet_engine::api::Caller {
    signet_engine::api::Caller {
        id: c.id.clone(),
        tenant: c.tenant.clone(),
        groups: c.groups.clone(),
        roles: c.roles.clone(),
        attrs: c.attrs.clone(),
    }
}

fn engine_error_to_tool_error(e: EngineError) -> ToolError {
    match e {
        EngineError::NotFound => ToolError::NotFound,
        EngineError::PolicyDenied { .. } => ToolError::PolicyDenied,
        EngineError::InvalidArgument(m) => ToolError::InvalidArgs(m),
        EngineError::Backend(m) => ToolError::Backend(m),
        EngineError::UnknownSchema { namespace, schema } => {
            ToolError::InvalidArgs(format!("unknown schema `{namespace}.{schema}`"))
        }
    }
}

// ---------------------------------------------------------------------------
// Stub engine — useful for integration tests of the gateway in isolation.
// Replaced with `signet_engine::Engine` in production. Feature-gated so it does
// not leak into release binaries unless explicitly requested.
// ---------------------------------------------------------------------------

/// A facade that returns deterministic fake data. Only compiled in for tests
/// or when the `stub-engine` feature is enabled.
#[cfg(any(test, feature = "stub-engine"))]
pub struct StubEngineFacade;

#[cfg(any(test, feature = "stub-engine"))]
#[async_trait]
impl EngineFacade for StubEngineFacade {
    async fn search(
        &self,
        _caller: &signet_engine::api::Caller,
        _schema: &str,
        query: &str,
        _top_k: usize,
        _filters: &Value,
    ) -> Result<Value, EngineError> {
        Ok(json!({
            "receipt_id": "stub_receipt",
            "results": [
                { "id": "stub_1", "score": 0.42, "excerpt": format!("stub match for `{query}`"), "record": {} }
            ]
        }))
    }
    async fn get(
        &self,
        _caller: &signet_engine::api::Caller,
        _schema: &str,
        id: &Value,
    ) -> Result<Value, EngineError> {
        Ok(json!({ "id": id, "note": "stub" }))
    }
    async fn list(
        &self,
        _caller: &signet_engine::api::Caller,
        _schema: &str,
        _filters: &Value,
        _limit: usize,
        _cursor: Option<&str>,
        _order_by: Option<&str>,
        _direction: &str,
    ) -> Result<Value, EngineError> {
        Ok(json!({ "items": [], "next_cursor": null }))
    }
    async fn list_relation(
        &self,
        _caller: &signet_engine::api::Caller,
        _schema: &str,
        _relation: &str,
        _parent_id: &Value,
        _limit: usize,
        _cursor: Option<&str>,
    ) -> Result<Value, EngineError> {
        Ok(json!({ "items": [], "next_cursor": null }))
    }
    async fn aggregate(
        &self,
        _caller: &signet_engine::api::Caller,
        _schema: &str,
        _facet: &str,
        _filters: &Value,
        _buckets: usize,
    ) -> Result<Value, EngineError> {
        Ok(json!({ "buckets": [] }))
    }
}
