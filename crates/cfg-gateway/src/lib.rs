//! ContextForge HTTP gateway.
//!
//! Hosts the REST API, the MCP JSON-RPC over HTTP endpoints, and the OAuth 2.1
//! metadata surface. All business logic lives in `cfg-engine`; this crate is thin.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod auth;
pub mod config;
pub mod mcp;
pub mod rest;
pub mod server;

pub use server::{AppState, build_app};
