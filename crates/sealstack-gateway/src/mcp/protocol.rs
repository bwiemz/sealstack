//! JSON-RPC dispatcher for MCP methods.
//!
//! Handles the four methods we ship in v0.1:
//! * `initialize`
//! * `tools/list`
//! * `tools/call`
//! * `resources/list`
//!
//! Everything else returns `METHOD_NOT_FOUND`.

use serde_json::{Value, json};

use super::registry::{ToolError, ToolRegistry};
use super::types::{
    Caller, InitializeParams, InitializeResult, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    ResourcesCapability, ServerCapabilities, ServerInfo, ToolContent, ToolsCallParams,
    ToolsCallResult, ToolsCapability, ToolsListResult,
};

/// Protocol revision this gateway advertises.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// The gateway's server identity block.
#[must_use]
pub fn server_info() -> ServerInfo {
    ServerInfo {
        name: "sealstack-gateway".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

/// Dispatch a single JSON-RPC request.
///
/// `server_name` identifies which MCP server the request arrived at (derived from the
/// URL path at `/mcp/<namespace>.<schema>`).
#[tracing::instrument(skip(registry, caller, req), fields(method = %req.method))]
pub async fn dispatch(
    server_name: &str,
    registry: &ToolRegistry,
    caller: &Caller,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    // Notification (id absent) → no response expected.
    let id = req.id.clone()?;

    let result = match req.method.as_str() {
        "initialize" => handle_initialize(req.params),
        "tools/list" => handle_tools_list(server_name, registry),
        "tools/call" => handle_tools_call(server_name, registry, caller, req.params).await,
        "resources/list" => handle_resources_list(server_name),
        "ping" => Ok(json!({})),
        other => Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!("method `{other}` is not supported by SealStack v0.1"),
        )),
    };

    Some(match result {
        Ok(v) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(v),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(e),
        },
    })
}

fn handle_initialize(params: Option<Value>) -> Result<Value, JsonRpcError> {
    let _params: InitializeParams = match params {
        Some(v) => serde_json::from_value(v)
            .map_err(|e| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, e.to_string()))?,
        None => {
            return Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                "initialize requires params",
            ));
        }
    };

    // We could negotiate protocol version here; for now we advertise the fixed one we support.
    let result = InitializeResult {
        protocol_version: PROTOCOL_VERSION.to_string(),
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability { list_changed: true }),
            resources: Some(ResourcesCapability {
                subscribe: false,
                list_changed: true,
            }),
            prompts: None,
            logging: Some(json!({})),
        },
        server_info: server_info(),
    };
    Ok(serde_json::to_value(result).unwrap())
}

fn handle_tools_list(server_name: &str, registry: &ToolRegistry) -> Result<Value, JsonRpcError> {
    let mut tools = registry.list_for(server_name);
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    serde_json::to_value(ToolsListResult { tools })
        .map_err(|e| JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, e.to_string()))
}

async fn handle_tools_call(
    server_name: &str,
    registry: &ToolRegistry,
    caller: &Caller,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = match params {
        Some(v) => serde_json::from_value(v)
            .map_err(|e| JsonRpcError::new(JsonRpcError::INVALID_PARAMS, e.to_string()))?,
        None => {
            return Err(JsonRpcError::new(
                JsonRpcError::INVALID_PARAMS,
                "tools/call requires params",
            ));
        }
    };

    let Some(handler) = registry.get(server_name, &params.name) else {
        return Err(JsonRpcError::new(
            JsonRpcError::METHOD_NOT_FOUND,
            format!(
                "tool `{}` not found on server `{}`",
                params.name, server_name
            ),
        ));
    };

    match handler.invoke(caller, &params.arguments).await {
        Ok(payload) => {
            let result = ToolsCallResult {
                content: vec![ToolContent::Json {
                    json: payload.clone(),
                }],
                is_error: false,
                structured_content: Some(payload),
            };
            Ok(serde_json::to_value(result).unwrap())
        }
        Err(ToolError::InvalidArgs(msg)) => {
            Err(JsonRpcError::new(JsonRpcError::INVALID_PARAMS, msg))
        }
        Err(ToolError::Unauthorized) => Err(JsonRpcError::new(
            JsonRpcError::UNAUTHORIZED,
            "unauthorized",
        )),
        Err(ToolError::PolicyDenied) => Err(JsonRpcError::new(
            JsonRpcError::POLICY_DENIED,
            "policy denied",
        )),
        Err(ToolError::NotFound) => Err(JsonRpcError::new(-32004, "not found")),
        Err(ToolError::Unimplemented) => Err(JsonRpcError::new(-32010, "unimplemented")),
        Err(ToolError::Backend(msg)) => Err(JsonRpcError::new(JsonRpcError::INTERNAL_ERROR, msg)),
    }
}

fn handle_resources_list(_server_name: &str) -> Result<Value, JsonRpcError> {
    // v0.1: resources are defined statically per schema; we'd fetch them from the schema
    // registry here. For the skeleton, return an empty list.
    Ok(json!({ "resources": [] }))
}
