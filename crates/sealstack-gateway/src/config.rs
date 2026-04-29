//! Gateway configuration.

use serde::{Deserialize, Serialize};

use crate::auth::AuthMode;
use crate::mcp::oauth::OAuthMetadataConfig;

/// Root gateway config. Populated from env in `main` via `Config::from_env`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Socket address to bind. Default: `0.0.0.0:7070`.
    pub bind: String,
    /// Postgres connection string.
    pub database_url: String,
    /// Qdrant gRPC URL.
    pub qdrant_url: String,
    /// Optional Redis URL for rate limiting + sessions.
    pub redis_url: Option<String>,
    /// OAuth metadata.
    pub oauth: OAuthMetadataConfig,
    /// Default log filter (e.g., `info,sealstack_gateway=debug`).
    pub log_filter: String,
    /// Bearer-token enforcement mode applied to `/mcp` routes.
    #[serde(skip, default = "default_auth_mode")]
    pub auth: AuthMode,
}

fn default_auth_mode() -> AuthMode {
    AuthMode::Disabled
}

impl Config {
    /// Load from environment. Panics on missing required values in production mode.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            bind: std::env::var("SEALSTACK_BIND").unwrap_or_else(|_| "0.0.0.0:7070".into()),
            database_url: std::env::var("SEALSTACK_DATABASE_URL").unwrap_or_else(|_| {
                "postgres://sealstack:sealstack@localhost:5432/sealstack".into()
            }),
            qdrant_url: std::env::var("SEALSTACK_QDRANT_URL")
                .unwrap_or_else(|_| "http://localhost:6334".into()),
            redis_url: std::env::var("SEALSTACK_REDIS_URL").ok(),
            oauth: OAuthMetadataConfig::dev_default(),
            log_filter: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
            auth: AuthMode::from_env(),
        }
    }

    /// Config suitable for an in-process integration test.
    #[must_use]
    pub fn test() -> Self {
        Self {
            bind: "127.0.0.1:0".into(),
            database_url: std::env::var("SEALSTACK_DATABASE_URL").unwrap_or_else(|_| {
                "postgres://sealstack:sealstack@localhost:5432/sealstack".into()
            }),
            qdrant_url: std::env::var("SEALSTACK_QDRANT_URL")
                .unwrap_or_else(|_| "http://localhost:6334".into()),
            redis_url: None,
            oauth: OAuthMetadataConfig::dev_default(),
            log_filter: "debug".into(),
            auth: AuthMode::Disabled,
        }
    }
}
