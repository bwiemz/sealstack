//! Public REST API.
//!
//! Every endpoint is a thin adapter: parse a JSON body, call the engine,
//! serialize the result. Business logic lives in `signet-engine`; connector
//! orchestration lives in `signet-ingest`.
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

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use signet_engine::api::{Caller, EngineError, EngineHandle, SearchRequest};
use signet_engine::schema_registry::SchemaMeta;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::AppState;

/// Assemble the REST routes.
#[must_use]
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/query", post(post_query))
        .route("/v1/schemas", get(list_schemas).post(register_schema))
        .route("/v1/schemas/:qualified", get(get_schema))
        .route("/v1/schemas/:qualified/ddl", post(apply_schema_ddl))
        .route(
            "/v1/connectors",
            get(list_connectors).post(register_connector),
        )
        .route("/v1/connectors/:id/sync", post(sync_connector))
        .route("/v1/receipts/:id", get(get_receipt))
}

// ---------------------------------------------------------------------------
// Liveness / readiness
// ---------------------------------------------------------------------------

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn readyz(State(_state): State<AppState>) -> impl IntoResponse {
    // TODO: DB + vector-store ping. v0.1 reports ready if the process is up.
    (StatusCode::OK, Json(json!({ "status": "ready" })))
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct QueryBody {
    schema: String,
    query: String,
    #[serde(default)]
    top_k: Option<usize>,
    #[serde(default)]
    filters: Value,
}

async fn post_query(
    State(state): State<AppState>,
    caller: CallerExt,
    Json(body): Json<QueryBody>,
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
        top_k: body.top_k.unwrap_or(0),
        filters: body.filters,
    };

    match state.engine.search(req).await {
        Ok(resp) => ok(json!(resp)),
        Err(e) => engine_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RegisterSchemaBody {
    /// Schema metadata JSON as emitted by `signet_csl::codegen`
    /// (the contents of `out/schemas/<ns>.<name>.schema.json`).
    meta: Value,
}

async fn register_schema(
    State(state): State<AppState>,
    Json(body): Json<RegisterSchemaBody>,
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
    crate::mcp::bootstrap::register_schema_tools(&state.registry, state.engine_facade.clone(), &meta);
    ok(json!({ "qualified": qualified, "status": "registered" }))
}

async fn list_schemas(State(state): State<AppState>) -> Response {
    let items: Vec<Value> = state
        .engine
        .registry()
        .iter()
        .into_iter()
        .map(|m| {
            json!({
                "namespace": m.namespace,
                "name":      m.name,
                "version":   m.version,
                "table":     m.table,
                "facets":    m.facets,
                "relations": m.relations.keys().collect::<Vec<_>>(),
            })
        })
        .collect();
    ok(json!({ "schemas": items }))
}

async fn get_schema(
    State(state): State<AppState>,
    Path(qualified): Path<String>,
) -> Response {
    let (ns, name) = match split_qualified(&qualified) {
        Ok(x) => x,
        Err(m) => return bad_request(m),
    };
    match state.engine.registry().get(&ns, &name) {
        Ok(meta) => ok(json!(*meta)),
        Err(e) => engine_error_response(e),
    }
}

#[derive(Debug, Deserialize)]
struct ApplyDdlBody {
    ddl: String,
}

async fn apply_schema_ddl(
    State(state): State<AppState>,
    Path(qualified): Path<String>,
    Json(body): Json<ApplyDdlBody>,
) -> Response {
    if let Err(m) = split_qualified(&qualified) {
        return bad_request(m);
    }
    match state.engine.store_handle().apply_schema_ddl(&body.ddl).await {
        Ok(()) => ok(json!({ "status": "applied" })),
        Err(e) => engine_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Connectors
// ---------------------------------------------------------------------------

async fn list_connectors(State(state): State<AppState>) -> Response {
    ok(json!({ "connectors": state.ingest_bindings() }))
}

#[derive(Debug, Deserialize)]
struct RegisterConnectorBody {
    kind: String,
    schema: String,
    #[serde(default)]
    config: Value,
}

async fn register_connector(
    State(state): State<AppState>,
    Json(body): Json<RegisterConnectorBody>,
) -> Response {
    let (namespace, name) = match split_qualified(&body.schema) {
        Ok(x) => x,
        Err(m) => return bad_request(m),
    };
    match state
        .register_connector(&body.kind, &namespace, &name, body.config)
        .await
    {
        Ok(id) => ok(json!({ "id": id, "status": "registered" })),
        Err(e) => anyhow_error_response(e),
    }
}

async fn sync_connector(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.sync_connector(&id).await {
        Some(outcome) => ok(json!(outcome)),
        None => not_found(format!("connector binding `{id}` not found")),
    }
}

// ---------------------------------------------------------------------------
// Receipts
// ---------------------------------------------------------------------------

async fn get_receipt(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.engine.receipts().fetch(&id).await {
        Ok(r) => ok(json!(r)),
        Err(e) => engine_error_response(e),
    }
}

// ---------------------------------------------------------------------------
// Caller extractor
// ---------------------------------------------------------------------------

/// Injects an authenticated [`Caller`] into each handler. In production,
/// sourced from the verified JWT; in v0.1 we accept an `X-Cfg-User` header
/// for local / CLI use and fall back to an anonymous caller.
struct CallerExt(Caller);

impl<S: Sync> axum::extract::FromRequestParts<S> for CallerExt {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Prefer the JWT-validated identity if the auth middleware ran.
        // Fall back to `X-Cfg-*` headers only when it didn't — i.e. on `/v1/*`
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
            .get("x-signet-user")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("anon")
            .to_owned();
        let tenant = parts
            .headers
            .get("x-signet-tenant")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let roles = parts
            .headers
            .get("x-signet-roles")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
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
        return Err(format!("schema `{qualified}` has an empty namespace or name"));
    }
    Ok((ns.to_owned(), name.to_owned()))
}
