//! Model Context Protocol gateway.
//!
//! # Architecture
//!
//! * `types`       — wire types for JSON-RPC 2.0 + MCP-specific structs.
//! * `registry`    — maps `namespace.schema.tool_name` → a `dyn ToolHandler`.
//! * `protocol`    — JSON-RPC dispatcher for `initialize`, `tools/list`, `tools/call`,
//!                   `resources/list`, `resources/read`, and lifecycle notifications.
//! * `transport`   — streamable-HTTP transport (per MCP 2025-11 spec): single POST
//!                   endpoint per server, optional `GET /sse` for server→client events,
//!                   session tokens in `Mcp-Session-Id` headers.
//! * `oauth`       — OAuth 2.1 well-known endpoints: `/.well-known/oauth-authorization-server`
//!                   and `/.well-known/oauth-protected-resource`.
//! * `handlers`    — the default `search_*`, `get_*`, `list_*` implementations that
//!                   delegate to `sealstack-engine`.
//!
//! The MCP gateway is mounted on the shared Axum router by `server::build_app`.
//!
//! A single `sealstack-gateway` process hosts one MCP server per CSL schema, each
//! addressable at `/mcp/<namespace>.<schema>`. A registry lookup at request time
//! decides which handler dispatches a given tool call.

pub mod bootstrap;
pub mod handlers;
pub mod oauth;
pub mod protocol;
pub mod registry;
pub mod transport;
pub mod types;

pub use registry::{ToolHandler, ToolRegistry};
pub use types::*;
