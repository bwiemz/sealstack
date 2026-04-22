//! SealStack HTTP gateway.
//!
//! Hosts the REST API, the MCP JSON-RPC over HTTP endpoints, and the OAuth 2.1
//! metadata surface. All business logic lives in `sealstack-engine`; this crate is thin.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod auth;
pub mod config;
pub mod mcp;
pub mod policy;
pub mod rest;
pub mod server;

pub use policy::policy_from_dir;
pub use server::{AppState, build_app};
