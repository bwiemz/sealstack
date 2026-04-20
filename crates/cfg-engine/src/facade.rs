//! Thin, JSON-shaped façade implemented on [`crate::Engine`] for the gateway.
//!
//! # Why two traits?
//!
//! [`crate::api::EngineHandle`] is the structured, typed interface — the REST
//! surface and Rust SDK call through it. [`EngineFacade`] is deliberately
//! *untyped*: each method takes primitives and returns [`serde_json::Value`].
//! The MCP gateway operates on JSON-RPC requests and responses, so a
//! JSON-in / JSON-out facade is a closer fit for its dispatch hot path.
//!
//! The facade layer also gives the gateway a crate boundary that does not know
//! about the engine's internal request/response structs, keeping its build
//! closure small.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::api::{
    AggregateRequest, Caller, EngineError, EngineHandle, GetRequest, ListRelationRequest,
    ListRequest, SearchRequest,
};

/// Thin facade the MCP gateway calls through.
///
/// An implementation of [`EngineFacade`] is provided for [`crate::Engine`]. Any
/// custom implementation (e.g. a mock for integration tests) must preserve the
/// error semantics described on each method.
#[async_trait]
pub trait EngineFacade: Send + Sync + 'static {
    /// Hybrid search over a schema's `@searchable` and `@chunked` fields.
    async fn search(
        &self,
        caller: &Caller,
        schema: &str,
        query: &str,
        top_k: usize,
        filters: &Value,
    ) -> Result<Value, EngineError>;

    /// Fetch a single record by primary key. `id` is a JSON value (string for
    /// Ulid/Uuid; numeric for I32/I64).
    async fn get(
        &self,
        caller: &Caller,
        schema: &str,
        id: &Value,
    ) -> Result<Value, EngineError>;

    /// Paginated list with facet filters. `cursor` is opaque.
    async fn list(
        &self,
        caller: &Caller,
        schema: &str,
        filters: &Value,
        limit: usize,
        cursor: Option<&str>,
        order_by: Option<&str>,
        direction: &str,
    ) -> Result<Value, EngineError>;

    /// Walk a `many` relation.
    async fn list_relation(
        &self,
        caller: &Caller,
        schema: &str,
        relation: &str,
        parent_id: &Value,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<Value, EngineError>;

    /// Facet histogram.
    async fn aggregate(
        &self,
        caller: &Caller,
        schema: &str,
        facet: &str,
        filters: &Value,
        buckets: usize,
    ) -> Result<Value, EngineError>;
}

/// Split a qualified schema `"namespace.Name"` into parts.
fn split_qualified(qualified: &str) -> Result<(String, String), EngineError> {
    // Split from the right on the last `.`. Anything before is the namespace;
    // the trailing segment is the schema.
    let (ns, name) = qualified.rsplit_once('.').ok_or_else(|| {
        EngineError::InvalidArgument(format!(
            "schema `{qualified}` is not qualified (expected `namespace.Name`)"
        ))
    })?;
    if ns.is_empty() || name.is_empty() {
        return Err(EngineError::InvalidArgument(format!(
            "schema `{qualified}` has an empty namespace or name"
        )));
    }
    Ok((ns.to_owned(), name.to_owned()))
}

fn id_to_string(id: &Value) -> Result<String, EngineError> {
    match id {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(EngineError::InvalidArgument(
            "id must be a string or number".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Blanket impl over anything that implements EngineHandle.
// ---------------------------------------------------------------------------

#[async_trait]
impl<E: EngineHandle> EngineFacade for E {
    async fn search(
        &self,
        caller: &Caller,
        schema: &str,
        query: &str,
        top_k: usize,
        filters: &Value,
    ) -> Result<Value, EngineError> {
        let (namespace, name) = split_qualified(schema)?;
        let resp = EngineHandle::search(
            self,
            SearchRequest {
                caller: caller.clone(),
                namespace,
                schema: name,
                query: query.to_owned(),
                top_k,
                filters: filters.clone(),
            },
        )
        .await?;
        Ok(json!({
            "receipt_id": resp.receipt_id,
            "results": resp.results,
        }))
    }

    async fn get(
        &self,
        caller: &Caller,
        schema: &str,
        id: &Value,
    ) -> Result<Value, EngineError> {
        let (namespace, name) = split_qualified(schema)?;
        let id = id_to_string(id)?;
        EngineHandle::get(
            self,
            GetRequest {
                caller: caller.clone(),
                namespace,
                schema: name,
                id,
            },
        )
        .await
    }

    async fn list(
        &self,
        caller: &Caller,
        schema: &str,
        filters: &Value,
        limit: usize,
        cursor: Option<&str>,
        _order_by: Option<&str>,
        _direction: &str,
    ) -> Result<Value, EngineError> {
        // v0.1: ignore order_by/direction; list() sorts on id ascending by
        // construction. order_by wires in when the registry surfaces per-schema
        // sortable fields.
        let (namespace, name) = split_qualified(schema)?;
        let resp = EngineHandle::list(
            self,
            ListRequest {
                caller: caller.clone(),
                namespace,
                schema: name,
                filters: filters.clone(),
                cursor: cursor.map(str::to_owned),
                limit,
            },
        )
        .await?;
        Ok(json!({
            "items": resp.items,
            "next_cursor": resp.next_cursor,
        }))
    }

    async fn list_relation(
        &self,
        caller: &Caller,
        schema: &str,
        relation: &str,
        parent_id: &Value,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<Value, EngineError> {
        let (namespace, name) = split_qualified(schema)?;
        let parent_id = id_to_string(parent_id)?;
        let resp = EngineHandle::list_relation(
            self,
            ListRelationRequest {
                caller: caller.clone(),
                namespace,
                schema: name,
                relation: relation.to_owned(),
                parent_id,
                cursor: cursor.map(str::to_owned),
                limit,
            },
        )
        .await?;
        Ok(json!({
            "items": resp.items,
            "next_cursor": resp.next_cursor,
        }))
    }

    async fn aggregate(
        &self,
        caller: &Caller,
        schema: &str,
        facet: &str,
        filters: &Value,
        buckets: usize,
    ) -> Result<Value, EngineError> {
        let (namespace, name) = split_qualified(schema)?;
        let resp = EngineHandle::aggregate(
            self,
            AggregateRequest {
                caller: caller.clone(),
                namespace,
                schema: name,
                facet: facet.to_owned(),
                filters: filters.clone(),
                limit: buckets,
            },
        )
        .await?;
        Ok(json!({ "buckets": resp.buckets }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_qualified_happy_path() {
        let (ns, name) = split_qualified("acme.crm.Customer").unwrap();
        assert_eq!(ns, "acme.crm");
        assert_eq!(name, "Customer");
    }

    #[test]
    fn split_qualified_rejects_unqualified() {
        assert!(split_qualified("Customer").is_err());
        assert!(split_qualified("").is_err());
        assert!(split_qualified(".Customer").is_err());
        assert!(split_qualified("acme.").is_err());
    }

    #[test]
    fn id_conversions() {
        assert_eq!(id_to_string(&json!("abc")).unwrap(), "abc");
        assert_eq!(id_to_string(&json!(42)).unwrap(), "42");
        assert!(id_to_string(&json!({"not": "allowed"})).is_err());
    }
}
