//! Shared types, errors, and identifiers used across every SealStack crate.
//!
//! Nothing in this crate depends on a runtime (no tokio, no sqlx, no reqwest).
//! It is deliberately dependency-light so the full workspace builds in tiers:
//! this crate first, then the runtime crates on top.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// The shared error type.
///
/// Backends (vector stores, embedders, connectors) return this. The engine
/// converts into `sealstack_engine::api::EngineError` at its surface; the gateway
/// converts into `sealstack_gateway::mcp::ToolError`. Each layer gets its own error
/// taxonomy because the wire-level meanings differ.
#[derive(Debug, thiserror::Error)]
pub enum SealStackError {
    /// Entity not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Caller is not authorized.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// Request arguments failed validation.
    #[error("invalid argument: {0}")]
    Validation(String),

    /// Downstream service (DB, vector store, network) failed.
    #[error("backend: {0}")]
    Backend(String),

    /// Policy predicate denied the action.
    #[error("policy denied: {0}")]
    Policy(String),

    /// Configuration is missing or malformed.
    #[error("config: {0}")]
    Config(String),

    /// Rate limit hit against a downstream vendor API.
    #[error("rate limited")]
    RateLimited,

    /// Generic error with a pass-through message.
    #[error("{0}")]
    Other(String),
}

impl SealStackError {
    /// Build a [`Backend`](Self::Backend) from any `Display`able.
    pub fn backend(e: impl fmt::Display) -> Self {
        Self::Backend(e.to_string())
    }

    /// Build a [`Config`](Self::Config) from any `Display`able.
    pub fn config(e: impl fmt::Display) -> Self {
        Self::Config(e.to_string())
    }

    /// Build a [`Validation`](Self::Validation) from any `Display`able.
    pub fn validation(e: impl fmt::Display) -> Self {
        Self::Validation(e.to_string())
    }
}

/// Result alias used across the workspace.
pub type SealStackResult<T> = Result<T, SealStackError>;

// ---------------------------------------------------------------------------
// Identifiers — newtypes over `ulid::Ulid`.
//
// Using newtypes keeps identifier spaces distinct at the type level so
// accidentally passing a `SchemaId` where a `TenantId` is expected becomes a
// compile error.
// ---------------------------------------------------------------------------

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub ulid::Ulid);

        impl $name {
            /// Generate a new random id.
            #[must_use]
            pub fn new() -> Self {
                Self(ulid::Ulid::new())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                ulid::Ulid::from_string(s).map(Self)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

id_newtype!(
    /// Stable identifier for one context record (Ticket, Customer, Doc, …).
    ContextId
);
id_newtype!(
    /// Stable identifier for a CSL schema at a specific version.
    SchemaId
);
id_newtype!(
    /// Identifier for a tenant / workspace.
    TenantId
);
id_newtype!(
    /// Identifier for a connector instance.
    ConnectorId
);
id_newtype!(
    /// Identifier for a single ingested chunk in the vector store.
    ChunkId
);

// ---------------------------------------------------------------------------
// Tenant
// ---------------------------------------------------------------------------

/// Tenant / workspace record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tenant {
    /// Identifier.
    pub id: TenantId,
    /// Human-readable slug (e.g. `"acme"`).
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Tenant-level feature flags and overrides.
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
    /// Creation timestamp (UTC).
    pub created_at: time::OffsetDateTime,
}

impl Tenant {
    /// Construct a new tenant with a fresh id and `created_at = now()`.
    #[must_use]
    pub fn new(slug: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: TenantId::new(),
            slug: slug.into(),
            name: name.into(),
            attrs: serde_json::Map::new(),
            created_at: time::OffsetDateTime::now_utc(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrip_string() {
        let a = ContextId::new();
        let s = a.to_string();
        let b: ContextId = s.parse().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn ids_are_distinct_types() {
        // This is a compile-time check; if the types ever merge, this stops compiling.
        fn takes_context(_: ContextId) {}
        let c = ContextId::new();
        takes_context(c);
        // takes_context(TenantId::new()); // would fail to compile — good.
    }

    #[test]
    fn tenant_constructor_sets_fresh_fields() {
        let a = Tenant::new("acme", "Acme Inc.");
        let b = Tenant::new("acme", "Acme Inc.");
        assert_ne!(a.id, b.id);
        assert_eq!(a.slug, "acme");
    }

    #[test]
    fn sealstack_error_display() {
        let e = SealStackError::Backend("boom".into());
        assert_eq!(format!("{e}"), "backend: boom");
    }
}
