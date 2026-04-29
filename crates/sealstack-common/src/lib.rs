//! Shared types, errors, and identifiers used across every `SealStack` crate.
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

    /// HTTP retry loop exhausted its attempt budget.
    #[error("retry exhausted after {attempts} attempts over {total_duration:?}: {last_error}")]
    RetryExhausted {
        /// Number of attempts made.
        attempts: u32,
        /// Wall time elapsed across all attempts.
        total_duration: std::time::Duration,
        /// Final error observed.
        last_error: Box<Self>,
    },

    /// Response body exceeded the configured size cap.
    #[error("response body exceeded cap: {cap_bytes} bytes")]
    BodyTooLarge {
        /// The cap that was exceeded, in bytes.
        cap_bytes: usize,
    },

    /// Paginator returned the same cursor twice consecutively.
    #[error("paginator cursor did not advance: {cursor}")]
    PaginatorCursorLoop {
        /// The repeated cursor value.
        cursor: String,
    },

    /// HTTP request produced a non-retryable status with headers + body the
    /// caller may need to inspect (e.g. GitHub's 403 discrimination).
    #[error("HTTP {status}")]
    HttpStatus {
        /// Status code.
        status: u16,
        /// Headers from the response, copied as `(name, value)` pairs.
        headers: Vec<(String, String)>,
        /// Body as UTF-8 text; empty if the body was non-UTF-8 or read
        /// failed. The body is already size-capped per the `HttpClient`'s
        /// configured cap.
        body: String,
    },
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

    #[test]
    fn retry_exhausted_renders_attempts_and_duration() {
        use std::time::Duration;
        let e = SealStackError::RetryExhausted {
            attempts: 5,
            total_duration: Duration::from_millis(7500),
            last_error: Box::new(SealStackError::Backend("502 bad gateway".into())),
        };
        let msg = e.to_string();
        assert!(msg.contains('5'), "missing attempts: {msg}");
        assert!(msg.contains("502"), "missing last_error detail: {msg}");
    }

    #[test]
    fn body_too_large_reports_cap() {
        let e = SealStackError::BodyTooLarge {
            cap_bytes: 52_428_800,
        };
        assert!(e.to_string().contains("52428800"));
    }

    #[test]
    fn paginator_cursor_loop_reports_cursor() {
        let e = SealStackError::PaginatorCursorLoop {
            cursor: "abc".into(),
        };
        assert!(e.to_string().contains("abc"));
    }
}
