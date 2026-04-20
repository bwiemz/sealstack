//! Tool registry: maps `(server_name, tool_name)` pairs to handler implementations.
//!
//! At startup, the gateway compiles every CSL schema in scope and auto-registers:
//! `search_X`, `get_X`, `list_X`, and one `list_X_<rel>` per `many` relation.
//! Each handler shares the same trait contract so the protocol dispatcher is uniform.

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use super::types::{Caller, ToolDescriptor};

/// Error shape for a tool invocation.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ToolError {
    /// The arguments failed JSON-Schema validation.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// The caller is not authorized for this tool.
    #[error("unauthorized")]
    Unauthorized,
    /// Policy evaluation denied access to the requested resource.
    #[error("policy denied")]
    PolicyDenied,
    /// Resource doesn't exist.
    #[error("not found")]
    NotFound,
    /// Backend (engine, vector store, DB) failed.
    #[error("backend error: {0}")]
    Backend(String),
    /// The tool is recognized but the action is not yet implemented.
    #[error("unimplemented")]
    Unimplemented,
}

/// Contract every MCP tool handler implements.
#[async_trait]
pub trait ToolHandler: Send + Sync + 'static {
    /// The descriptor advertised via `tools/list`.
    fn descriptor(&self) -> ToolDescriptor;

    /// Execute the tool. The caller is already authenticated by the transport layer.
    async fn invoke(&self, caller: &Caller, args: &Value) -> Result<Value, ToolError>;
}

/// The registry. Thread-safe, cheap to clone.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Keyed by `(server_name, tool_name)`.
    handlers: DashMap<(String, String), Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool under a given server.
    pub fn register(&self, server_name: impl Into<String>, handler: Arc<dyn ToolHandler>) {
        let desc = handler.descriptor();
        self.inner
            .handlers
            .insert((server_name.into(), desc.name.clone()), handler);
    }

    /// List every tool registered for a server, in registration order is not guaranteed;
    /// callers requiring stable ordering should sort on the returned descriptors.
    #[must_use]
    pub fn list_for(&self, server_name: &str) -> Vec<ToolDescriptor> {
        self.inner
            .handlers
            .iter()
            .filter(|e| e.key().0 == server_name)
            .map(|e| e.value().descriptor())
            .collect()
    }

    /// Look up a handler.
    #[must_use]
    pub fn get(&self, server_name: &str, tool_name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.inner
            .handlers
            .get(&(server_name.to_owned(), tool_name.to_owned()))
            .map(|e| e.value().clone())
    }
}
