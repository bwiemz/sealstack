//! Streamable HTTP transport for MCP (spec revision 2025-11-25).
//!
//! # Shape
//!
//! Each MCP server is mounted at `/mcp/<qualified-name>`. The route accepts:
//!
//! * `POST /mcp/<name>` — a single JSON-RPC request (or batch). The response is either
//!   a single JSON-RPC response body or an SSE stream (`text/event-stream`) when the
//!   handler elects to stream progress/notifications mid-call.
//!
//! * `GET /mcp/<name>` — optional SSE channel for server→client notifications
//!   (`notifications/tools/list_changed`, progress updates, etc.) outside of any
//!   specific call.
//!
//! * `DELETE /mcp/<name>` — ends a session early; servers are free to discard state.
//!
//! # Session tracking
//!
//! Clients send a `Mcp-Session-Id` header after `initialize`. We issue one on the
//! first successful initialize and echo it thereafter. Sessions are opaque ULIDs with
//! a configurable idle TTL (default 30 min).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use dashmap::DashMap;
use serde_json::Value;
use tracing::{debug, warn};

use super::protocol::dispatch;
use super::registry::ToolRegistry;
use super::types::{Caller, JsonRpcError, JsonRpcRequest, JsonRpcResponse};

/// Header name for session tracking per the spec.
pub const SESSION_HEADER: &str = "Mcp-Session-Id";

/// Default session idle TTL.
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// Session state.
#[derive(Clone, Debug)]
pub struct Session {
    /// Session id (ULID string).
    pub id: String,
    /// Created-at timestamp.
    pub created: Instant,
    /// Last-accessed timestamp. Sessions idle past TTL are reaped.
    pub last_seen: Instant,
    /// Caller identity captured at initialize time.
    pub caller: Caller,
    /// Protocol version negotiated with the client.
    pub protocol_version: String,
}

/// Transport-level state shared by every MCP route.
#[derive(Clone)]
pub struct TransportState {
    /// Tool registry populated at boot from every compiled schema.
    pub registry: ToolRegistry,
    /// Sessions keyed by session id.
    pub sessions: Arc<DashMap<String, Session>>,
    /// Session idle TTL.
    pub session_ttl: Duration,
}

impl TransportState {
    /// New transport state with default TTL.
    #[must_use]
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            sessions: Arc::new(DashMap::new()),
            session_ttl: DEFAULT_SESSION_TTL,
        }
    }
}

/// Build the router fragment that mounts `/mcp/{server_name}` endpoints.
///
/// The caller should nest this onto the main app router, typically under `/mcp`.
pub fn router(state: TransportState) -> Router {
    Router::new()
        .route(
            "/{server_name}",
            post(handle_post).get(handle_get).delete(handle_delete),
        )
        .with_state(state)
}

// --- POST handler ---------------------------------------------------------------------

async fn handle_post(
    State(state): State<TransportState>,
    Path(server_name): Path<String>,
    headers: HeaderMap,
    authenticated: axum::extract::Extension<crate::auth::AuthenticatedCaller>,
    Json(body): Json<Value>,
) -> Response {
    // Fetch or create a session.
    let (session_id, caller) = match resolve_session(&state, &headers, &authenticated.0) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };

    // Accept both single and batch JSON-RPC.
    let responses = match body {
        Value::Array(arr) => dispatch_batch(&state, &server_name, &caller, arr).await,
        single => match serde_json::from_value::<JsonRpcRequest>(single) {
            Ok(req) => match dispatch(&server_name, &state.registry, &caller, req).await {
                Some(resp) => vec![resp],
                None => return StatusCode::ACCEPTED.into_response(), // notification
            },
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(error_response(
                        Value::Null,
                        JsonRpcError::PARSE_ERROR,
                        e.to_string(),
                    )),
                )
                    .into_response();
            }
        },
    };

    let mut response = if responses.len() == 1 {
        Json(responses.into_iter().next().unwrap()).into_response()
    } else {
        Json(responses).into_response()
    };

    response.headers_mut().insert(
        SESSION_HEADER,
        HeaderValue::from_str(&session_id).unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn dispatch_batch(
    state: &TransportState,
    server_name: &str,
    caller: &Caller,
    arr: Vec<Value>,
) -> Vec<JsonRpcResponse> {
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        match serde_json::from_value::<JsonRpcRequest>(item) {
            Ok(req) => {
                if let Some(resp) = dispatch(server_name, &state.registry, caller, req).await {
                    out.push(resp);
                }
            }
            Err(e) => out.push(error_response(
                Value::Null,
                JsonRpcError::PARSE_ERROR,
                e.to_string(),
            )),
        }
    }
    out
}

// --- GET handler (SSE notification channel) -----------------------------------------

async fn handle_get(
    State(_state): State<TransportState>,
    Path(_server_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    // v0.1: we accept the connection but do not emit any server-initiated events.
    // Later we'll wire this into a per-session broadcast channel that carries
    // `notifications/tools/list_changed`, progress updates from long-running tools, etc.
    let session_id = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    debug!(session = %session_id, "SSE channel opened (v0.1 no-op)");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        // Empty body; clients will keep the connection open until we push events.
        "",
    )
        .into_response()
}

// --- DELETE handler (terminate session) ---------------------------------------------

async fn handle_delete(
    State(state): State<TransportState>,
    Path(_server_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(sid) = headers.get(SESSION_HEADER).and_then(|v| v.to_str().ok()) {
        state.sessions.remove(sid);
    }
    StatusCode::NO_CONTENT.into_response()
}

// --- Session resolution --------------------------------------------------------------

fn resolve_session(
    state: &TransportState,
    headers: &HeaderMap,
    authenticated: &crate::auth::AuthenticatedCaller,
) -> Result<(String, Caller), Response> {
    // 1. Reap expired sessions opportunistically.
    reap_expired(state);

    // 2. Pull the session id if supplied.
    let existing = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    if let Some(sid) = existing {
        if let Some(mut entry) = state.sessions.get_mut(&sid) {
            entry.last_seen = Instant::now();
            return Ok((entry.id.clone(), entry.caller.clone()));
        }
        warn!(session = %sid, "unknown session id; issuing a new one");
    }

    // 3. Issue a new session under the authenticated identity the middleware
    //    placed into request extensions. In `AuthMode::Disabled` that
    //    identity is `anonymous`; under `Hs256` it is the JWT subject.
    let caller = Caller {
        id: authenticated.id.clone(),
        tenant: authenticated.tenant.clone(),
        groups: Vec::new(),
        roles: authenticated.roles.clone(),
        attrs: authenticated.attrs.clone(),
    };
    let sid = ulid::Ulid::new().to_string();
    let now = Instant::now();
    state.sessions.insert(
        sid.clone(),
        Session {
            id: sid.clone(),
            created: now,
            last_seen: now,
            caller: caller.clone(),
            protocol_version: super::protocol::PROTOCOL_VERSION.to_string(),
        },
    );
    Ok((sid, caller))
}

fn reap_expired(state: &TransportState) {
    let now = Instant::now();
    state
        .sessions
        .retain(|_, s| now.duration_since(s.last_seen) < state.session_ttl);
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcError::new(code, message)),
    }
}
