//! Public REST API.
//!
//! Every endpoint is a thin adapter: parse a JSON body, call the engine,
//! serialize the result. Business logic lives in `sealstack-engine`; connector
//! orchestration lives in `sealstack-ingest`.
//!
//! # Endpoint summary
//!
//! | Method | Path                                   | Purpose                           |
//! |--------|----------------------------------------|-----------------------------------|
//! | `GET`  | `/healthz`                             | Liveness                          |
//! | `GET`  | `/readyz`                              | Readiness                         |
//! | `POST` | `/v1/query`                            | Hybrid search                     |
//! | `GET`  | `/v1/schemas`                          | List registered schemas           |
//! | `POST` | `/v1/schemas`                          | Register a compiled CSL schema    |
//! | `GET`  | `/v1/schemas/:qualified`               | Get one schema's metadata         |
//! | `POST` | `/v1/schemas/:qualified/ddl`           | Apply per-schema DDL              |
//! | `GET`  | `/v1/connectors`                       | List connector bindings           |
//! | `POST` | `/v1/connectors`                       | Register a binding                |
//! | `POST` | `/v1/connectors/:id/sync`              | Run one-shot sync                 |
//! | `GET`  | `/v1/receipts/:id`                     | Fetch one receipt                 |
//!
//! All JSON responses follow `{ "data": ..., "error": null }` on success or
//! `{ "data": null, "error": { "code": ..., "message": ... } }` on failure.
//!
//! # Wire types
//!
//! Request and success-response shapes come from `sealstack-api-types`. The
//! engine emits richer internal structs (`SearchResponse`, `SchemaMeta`,
//! `Receipt`, `ConnectorBindingInfo`, `SyncOutcome`); each is projected to
//! its wire counterpart by a small helper at the bottom of this file.
//! Keeping the wire shapes in `sealstack-api-types` is what lets the
//! TS/Python SDKs and the gateway share one source of truth.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sealstack_api_types::{
    connectors::{
        ConnectorBindingWire, ListConnectorsResponse, RegisterConnectorRequest,
        RegisterConnectorResponse, SyncConnectorResponse,
    },
    health::{HealthStatus, HealthStatusKind},
    query::{QueryHit, QueryRequest, QueryResponse},
    receipts::{ReceiptSource, ReceiptWire},
    schemas::{
        ApplyDdlRequest, ApplyDdlResponse, ListSchemasResponse, RegisterSchemaRequest,
        RegisterSchemaResponse, SchemaMetaWire,
    },
};
use sealstack_engine::api::{Caller, EngineError, EngineHandle, SearchRequest, SearchResponse};
use sealstack_engine::receipts::Receipt;
use sealstack_engine::schema_registry::SchemaMeta;
use sealstack_ingest::registry::ConnectorBindingInfo;
use serde_json::{Value, json};

use crate::server::AppState;

/// Default hybrid alpha for schemas whose CSL `context` block omitted it.
/// Mirrors `sealstack_engine::config::default_hybrid_alpha`; duplicated here
/// to avoid coupling the wire layer to engine config internals. Bump in
/// lockstep if the engine default ever changes.
const DEFAULT_HYBRID_ALPHA: f32 = 0.6;

/// Assemble the REST routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/query", post(post_query))
        .route("/v1/schemas", get(list_schemas).post(register_schema))
        .route("/v1/schemas/{qualified}", get(get_schema))
        .route("/v1/schemas/{qualified}/ddl", post(apply_schema_ddl))
        .route(
            "/v1/connectors",
            get(list_connectors).post(register_connector),
        )
        .route("/v1/connectors/{id}/sync", post(sync_connector))
        .route("/v1/receipts/{id}", get(get_receipt))
}

// ---------------------------------------------------------------------------
// Liveness / readiness
// ---------------------------------------------------------------------------

async fn healthz() -> Response {
    ok(json!(HealthStatus {
        status: HealthStatusKind::Ok,
    }))
}

async fn readyz(State(_state): State<AppState>) -> Response {
    // TODO: DB + vector-store ping. v0.1 reports ready if the process is up.
    ok(json!(HealthStatus {
        status: HealthStatusKind::Ok,
    }))
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

async fn post_query(
    State(state): State<AppState>,
    caller: CallerExt,
    Json(body): Json<QueryRequest>,
) -> Response {
    let (namespace, schema) = match split_qualified(&body.schema) {
        Ok(x) => x,
        Err(m) => return bad_request(m),
    };
    let req = SearchRequest {
        caller: caller.0,
        namespace,
        schema,
        query: body.query,
        top_k: body.top_k.map(|k| k as usize).unwrap_or(0),
        filters: body.filters.unwrap_or(Value::Null),
    };

    match state.engine.search(req).await {
        Ok(resp) => ok(json!(search_response_to_wire(resp))),
        Err(e) => engine_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

async fn register_schema(
    State(state): State<AppState>,
    Json(body): Json<RegisterSchemaRequest>,
) -> Response {
    let meta: SchemaMeta = match serde_json::from_value(body.meta) {
        Ok(m) => m,
        Err(e) => return bad_request(format!("invalid schema meta: {e}")),
    };
    let qualified = format!("{}.{}", meta.namespace, meta.name);

    // Persist first; if the DB write fails we want the in-memory registry to
    // stay consistent with what a restarting gateway will rehydrate.
    if let Err(e) = state.engine.store_handle().put_schema(&meta).await {
        return engine_error_response(e);
    }
    state.engine.registry().insert(meta.clone());
    crate::mcp::bootstrap::register_schema_tools(
        &state.registry,
        state.engine_facade.clone(),
        &meta,
    );
    ok(json!(RegisterSchemaResponse { qualified }))
}

async fn list_schemas(State(state): State<AppState>) -> Response {
    let schemas: Vec<SchemaMetaWire> = state
        .engine
        .registry()
        .iter()
        .into_iter()
        .map(|m| schema_meta_to_wire((*m).clone()))
        .collect();
    ok(json!(ListSchemasResponse { schemas }))
}

async fn get_schema(State(state): State<AppState>, Path(qualified): Path<String>) -> Response {
    let (ns, name) = match split_qualified(&qualified) {
        Ok(x) => x,
        Err(m) => return bad_request(m),
    };
    match state.engine.registry().get(&ns, &name) {
        Ok(meta) => ok(json!(schema_meta_to_wire((*meta).clone()))),
        Err(e) => engine_error_response(e),
    }
}

async fn apply_schema_ddl(
    State(state): State<AppState>,
    Path(qualified): Path<String>,
    Json(body): Json<ApplyDdlRequest>,
) -> Response {
    if let Err(m) = split_qualified(&qualified) {
        return bad_request(m);
    }
    // `Store::apply_schema_ddl` returns `()` rather than a count. Naively
    // estimate the number of statements applied by counting top-level `;`
    // separators in the submitted DDL — close enough for the SDK's
    // diagnostic-grade `applied` field. Empty bodies report 0.
    let applied = count_ddl_statements(&body.ddl);
    match state
        .engine
        .store_handle()
        .apply_schema_ddl(&body.ddl)
        .await
    {
        Ok(()) => ok(json!(ApplyDdlResponse { applied })),
        Err(e) => engine_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Connectors
// ---------------------------------------------------------------------------

async fn list_connectors(State(state): State<AppState>) -> Response {
    let connectors: Vec<ConnectorBindingWire> = state
        .ingest_bindings()
        .into_iter()
        .map(connector_binding_to_wire)
        .collect();
    ok(json!(ListConnectorsResponse { connectors }))
}

async fn register_connector(
    State(state): State<AppState>,
    Json(body): Json<RegisterConnectorRequest>,
) -> Response {
    let (namespace, name) = match split_qualified(&body.schema) {
        Ok(x) => x,
        Err(m) => return bad_request(m),
    };
    match state
        .register_connector(&body.kind, &namespace, &name, body.config)
        .await
    {
        Ok(id) => ok(json!(RegisterConnectorResponse { id })),
        Err(e) => anyhow_error_response(e),
    }
}

async fn sync_connector(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.sync_connector(&id).await {
        Some(outcome) => ok(json!(SyncConnectorResponse {
            // v0.3 has no separate job table; the binding id uniquely names
            // the in-flight sync. SDKs treat job_id as opaque.
            job_id: outcome.binding_id,
        })),
        None => not_found(format!("connector binding `{id}` not found")),
    }
}

// ---------------------------------------------------------------------------
// Receipts
// ---------------------------------------------------------------------------

async fn get_receipt(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.engine.receipts().fetch(&id).await {
        Ok(r) => ok(json!(receipt_to_wire(r))),
        Err(e) => engine_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Caller extractor
// ---------------------------------------------------------------------------

/// Injects an authenticated [`Caller`] into each handler. In production,
/// sourced from the verified JWT; in v0.1 we accept an `X-Sealstack-User` header
/// for local / CLI use and fall back to an anonymous caller.
struct CallerExt(Caller);

impl<S: Sync> axum::extract::FromRequestParts<S> for CallerExt {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Prefer the JWT-validated identity if the auth middleware ran.
        // Fall back to `X-Sealstack-*` headers only when it didn't — i.e. on `/v1/*`
        // routes in dev. The MCP router always has the middleware attached,
        // so those requests arrive here (if at all) with the extension set.
        if let Some(a) = parts
            .extensions
            .get::<crate::auth::AuthenticatedCaller>()
            .cloned()
        {
            return Ok(Self(Caller {
                id: a.id,
                tenant: a.tenant,
                groups: vec![],
                roles: a.roles,
                attrs: a.attrs,
            }));
        }
        let user = parts
            .headers
            .get("x-sealstack-user")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anon")
            .to_owned();
        let tenant = parts
            .headers
            .get("x-sealstack-tenant")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let roles = parts
            .headers
            .get("x-sealstack-roles")
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self(Caller {
            id: user,
            tenant,
            groups: vec![],
            roles,
            attrs: serde_json::Map::new(),
        }))
    }
}

// ---------------------------------------------------------------------------
// Envelope helpers
// ---------------------------------------------------------------------------

fn ok(data: Value) -> Response {
    (StatusCode::OK, Json(json!({ "data": data, "error": null }))).into_response()
}

fn bad_request(message: impl Into<String>) -> Response {
    let body = json!({
        "data":  null,
        "error": { "code": "invalid_argument", "message": message.into() },
    });
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

fn not_found(message: impl Into<String>) -> Response {
    let body = json!({
        "data":  null,
        "error": { "code": "not_found", "message": message.into() },
    });
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}

fn engine_error_response(e: EngineError) -> Response {
    let (status, code) = match &e {
        EngineError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        EngineError::PolicyDenied { .. } => (StatusCode::FORBIDDEN, "policy_denied"),
        EngineError::InvalidArgument(_) => (StatusCode::BAD_REQUEST, "invalid_argument"),
        EngineError::UnknownSchema { .. } => (StatusCode::NOT_FOUND, "unknown_schema"),
        EngineError::Backend(_) => (StatusCode::INTERNAL_SERVER_ERROR, "backend"),
    };
    let body = json!({
        "data":  null,
        "error": { "code": code, "message": e.to_string() },
    });
    (status, Json(body)).into_response()
}

fn anyhow_error_response(e: anyhow::Error) -> Response {
    let body = json!({
        "data":  null,
        "error": { "code": "internal", "message": e.to_string() },
    });
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

fn split_qualified(qualified: &str) -> Result<(String, String), String> {
    let (ns, name) = qualified
        .rsplit_once('.')
        .ok_or_else(|| format!("schema `{qualified}` must be qualified as `namespace.Name`"))?;
    if ns.is_empty() || name.is_empty() {
        return Err(format!(
            "schema `{qualified}` has an empty namespace or name"
        ));
    }
    Ok((ns.to_owned(), name.to_owned()))
}

// ---------------------------------------------------------------------------
// Wire conversion helpers
//
// One helper per engine-internal type. Each is a pure field-mapping; no
// logic, no fallible paths beyond the field projections themselves.
// ---------------------------------------------------------------------------

/// Project `sealstack_engine::api::SearchResponse` to its wire shape.
fn search_response_to_wire(resp: SearchResponse) -> QueryResponse {
    QueryResponse {
        receipt_id: resp.receipt_id,
        results: resp
            .results
            .into_iter()
            .map(|h| QueryHit {
                id: h.id,
                score: h.score,
                excerpt: h.excerpt,
                record: h.record,
            })
            .collect(),
    }
}

/// Project `sealstack_engine::SchemaMeta` to its wire shape. Drops `fields`,
/// `relations`, `chunked_fields`, `facets`, and `context`; flattens
/// `Option<f32> hybrid_alpha` to `f32` using `DEFAULT_HYBRID_ALPHA` when
/// the schema declined to override it.
fn schema_meta_to_wire(meta: SchemaMeta) -> SchemaMetaWire {
    SchemaMetaWire {
        namespace: meta.namespace,
        name: meta.name,
        version: meta.version,
        primary_key: meta.primary_key,
        table: meta.table,
        collection: meta.collection,
        hybrid_alpha: meta.hybrid_alpha.unwrap_or(DEFAULT_HYBRID_ALPHA),
    }
}

/// Project `sealstack_ingest::ConnectorBindingInfo` to its wire shape.
///
/// Renames `connector` → `kind` for SDK ergonomics. v0.3 has no disable
/// surface, so every registered binding is reported as `enabled: true`;
/// when the gateway grows a "pause binding" endpoint this will read from
/// the binding's runtime state instead.
fn connector_binding_to_wire(info: ConnectorBindingInfo) -> ConnectorBindingWire {
    ConnectorBindingWire {
        id: info.id,
        kind: info.connector,
        schema: format!("{}.{}", info.namespace, info.schema),
        enabled: true,
    }
}

/// Project `sealstack_engine::receipts::Receipt` to its wire shape. Drops
/// `qualified_schema`, `tool`, `arguments`, `policies_applied`, and
/// `timings_ms`; renames `created_at` → `issued_at`; lifts `caller.tenant`
/// to a top-level `tenant` field; maps each `SourceRef` → `ReceiptSource`.
fn receipt_to_wire(r: Receipt) -> ReceiptWire {
    ReceiptWire {
        id: r.id,
        caller_id: r.caller.id,
        tenant: r.caller.tenant,
        sources: r
            .sources
            .into_iter()
            .map(|s| ReceiptSource {
                // Engine's `chunk_id` is `Option<String>`; the wire shape
                // wants a non-optional string. Empty string when missing
                // is the closest "no chunk" signal we can give the SDK
                // without adding optionality across the boundary.
                chunk_id: s.chunk_id.unwrap_or_default(),
                // Engine's `SourceRef` exposes `{schema, record_id}` but
                // not a fully-qualified URI. v0.3 stitches them into a
                // synthetic `<schema>:<record_id>` URI; v0.4 will replace
                // this with a real URI once connectors record one.
                source_uri: format!("{}:{}", s.schema, s.record_id),
                score: s.score,
            })
            .collect(),
        issued_at: r.created_at,
    }
}

/// Naive top-level statement counter for the DDL applied to a schema.
/// Splits on `;` outside of single-quoted and dollar-quoted strings.
/// Used only for the `applied` field of [`ApplyDdlResponse`]; the engine
/// itself runs the DDL through a stricter splitter.
fn count_ddl_statements(ddl: &str) -> u32 {
    let mut count: u32 = 0;
    let mut current_has_text = false;
    let mut in_str = false;
    let mut dollar_tag: Option<String> = None;
    let bytes = ddl.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if !in_str && dollar_tag.is_none() && c == '-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if !in_str && dollar_tag.is_none() && c == '$' {
            // Try to read a $tag$ marker.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'$' {
                j += 1;
            }
            if j < bytes.len() {
                let tag = ddl[i..=j].to_owned();
                dollar_tag = Some(tag);
                current_has_text = true;
                i = j + 1;
                continue;
            }
        }
        if let Some(tag) = &dollar_tag {
            if ddl[i..].starts_with(tag) {
                i += tag.len();
                dollar_tag = None;
                continue;
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            in_str = !in_str;
            current_has_text = true;
            i += 1;
            continue;
        }
        if !in_str && c == ';' {
            if current_has_text {
                count += 1;
            }
            current_has_text = false;
            i += 1;
            continue;
        }
        if !c.is_whitespace() {
            current_has_text = true;
        }
        i += 1;
    }
    if current_has_text {
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_ddl_statements_handles_typical_bundle() {
        assert_eq!(count_ddl_statements(""), 0);
        assert_eq!(count_ddl_statements("CREATE TABLE foo();"), 1);
        assert_eq!(
            count_ddl_statements("CREATE TABLE foo(); CREATE INDEX i ON foo(x);"),
            2
        );
        // Trailing statement without `;` still counts.
        assert_eq!(count_ddl_statements("CREATE TABLE foo()"), 1);
        // Comments and blank space don't add phantom statements.
        assert_eq!(count_ddl_statements("-- nothing here\n   \n"), 0);
    }
}
