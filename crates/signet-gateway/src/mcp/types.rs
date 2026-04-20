//! Wire types for JSON-RPC 2.0 and the Model Context Protocol (spec revision 2025-11-25).
//!
//! These are **wire** structures — intentionally untyped where the spec allows variance
//! (tool params, result payloads). Downstream code converts them into typed engine calls.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request envelope.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    /// Always the string `"2.0"`.
    pub jsonrpc: String,
    /// Request id. `None` for notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    /// Method name, e.g., `"tools/list"`.
    pub method: String,
    /// Method parameters; shape varies per method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JsonRpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Echo of the corresponding request id.
    pub id: Value,
    /// Present on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Present on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// Structured error object.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JsonRpcError {
    /// Error code. Standard JSON-RPC codes plus MCP-specific extensions.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Optional structured data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// -32700
    pub const PARSE_ERROR: i64 = -32700;
    /// -32600
    pub const INVALID_REQUEST: i64 = -32600;
    /// -32601
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// -32602
    pub const INVALID_PARAMS: i64 = -32602;
    /// -32603
    pub const INTERNAL_ERROR: i64 = -32603;
    /// MCP: caller not authorized for this tool.
    pub const UNAUTHORIZED: i64 = -32001;
    /// MCP: policy denied the specific resource.
    pub const POLICY_DENIED: i64 = -32002;

    /// Construct an error response.
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

// === MCP-specific messages ============================================================

/// `initialize` params from the client.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InitializeParams {
    /// Protocol revision the client expects to speak.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Capabilities the client exposes.
    #[serde(default)]
    pub capabilities: ClientCapabilities,
    /// Client info for logging.
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

/// `initialize` result returned to the client.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InitializeResult {
    /// Protocol revision the server supports.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server capabilities.
    pub capabilities: ServerCapabilities,
    /// Human info.
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

/// Identifying details a client sends during handshake.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ClientInfo {
    /// Software name.
    pub name: String,
    /// Version.
    pub version: String,
}

/// Identifying details a server advertises.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServerInfo {
    /// Software name.
    pub name: String,
    /// Version.
    pub version: String,
}

/// Capability bitset from the client.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ClientCapabilities {
    /// Whether the client supports `sampling/*` methods.
    #[serde(default)]
    pub sampling: Option<Value>,
    /// Whether the client supports `roots/*`.
    #[serde(default)]
    pub roots: Option<Value>,
    /// Generic unstructured extras.
    #[serde(flatten)]
    pub extras: serde_json::Map<String, Value>,
}

/// Capability bitset from the server.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServerCapabilities {
    /// Whether the server serves tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Whether the server serves resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    /// Whether the server serves prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    /// Logging support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Value>,
}

/// `tools` capability descriptor.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ToolsCapability {
    /// Whether `notifications/tools/list_changed` is supported.
    #[serde(default, rename = "listChanged")]
    pub list_changed: bool,
}

/// `resources` capability descriptor.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ResourcesCapability {
    /// Whether `resources/subscribe` is supported.
    #[serde(default)]
    pub subscribe: bool,
    /// Whether `notifications/resources/list_changed` is supported.
    #[serde(default, rename = "listChanged")]
    pub list_changed: bool,
}

/// `prompts` capability descriptor.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PromptsCapability {
    /// Whether `notifications/prompts/list_changed` is supported.
    #[serde(default, rename = "listChanged")]
    pub list_changed: bool,
}

/// A single tool descriptor, as returned by `tools/list`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ToolDescriptor {
    /// Tool id used in `tools/call`.
    pub name: String,
    /// Optional display title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Human description.
    pub description: String,
    /// JSON Schema for input arguments.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// Optional JSON Schema for the result payload.
    #[serde(default, rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Optional free-form annotations; not interpreted by the protocol but surfaced to clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

/// `tools/list` result.
#[derive(Clone, Debug, Serialize)]
pub struct ToolsListResult {
    /// All tools this server exposes for the current session.
    pub tools: Vec<ToolDescriptor>,
}

/// `tools/call` params.
#[derive(Clone, Debug, Deserialize)]
pub struct ToolsCallParams {
    /// Name of the tool to invoke.
    pub name: String,
    /// Arguments; validated against the tool's `inputSchema`.
    #[serde(default)]
    pub arguments: Value,
}

/// Result of a `tools/call`.
#[derive(Clone, Debug, Serialize)]
pub struct ToolsCallResult {
    /// Content blocks to render. Typically one `json` block.
    pub content: Vec<ToolContent>,
    /// `true` if the tool errored at the application level (vs. protocol level).
    #[serde(rename = "isError", default)]
    pub is_error: bool,
    /// Optional structured payload for clients that prefer typed results.
    #[serde(default, rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
}

/// A single content block in a tool result.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    /// Plain text.
    Text {
        /// Text body.
        text: String,
    },
    /// JSON payload (serialized as text by clients that need text).
    Json {
        /// The payload.
        json: Value,
    },
    /// Inline image, base64-encoded.
    Image {
        /// Base64-encoded content.
        data: String,
        /// MIME type.
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

// === Caller context the gateway injects before dispatching tools ====================

/// Authenticated caller context — injected from OAuth introspection or bearer JWT.
#[derive(Clone, Debug, Default)]
pub struct Caller {
    /// Stable principal id, e.g. `user:abc123`.
    pub id: String,
    /// Tenant/workspace ULID as a string.
    pub tenant: String,
    /// Identity groups/teams.
    pub groups: Vec<String>,
    /// Assigned roles.
    pub roles: Vec<String>,
    /// Free-form attributes (region, department, etc.).
    pub attrs: serde_json::Map<String, Value>,
}

impl Caller {
    /// Whether the caller has a named role.
    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
}
