//! Command handlers.
//!
//! One module per top-level subcommand. Each module exposes a single
//! `run(&Context, args) -> anyhow::Result<()>` function.

use std::path::PathBuf;

use crate::output::Format;

/// Shared context threaded through every handler.
pub(crate) struct Context {
    pub gateway_url: String,
    pub user: Option<String>,
    pub project_root: PathBuf,
    pub format: Format,
}

impl Context {
    /// Resolve the caller id: `--user` > `$USER` > `"anon"`.
    pub(crate) fn effective_user(&self) -> String {
        self.user
            .clone()
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_else(|| "anon".to_string())
    }

    /// Build an HTTP client against the configured gateway.
    pub(crate) fn client(&self) -> crate::client::Client {
        crate::client::Client::new(self.gateway_url.clone(), self.effective_user())
    }
}

pub(crate) mod compile;
pub(crate) mod connector;
pub(crate) mod dev;
pub(crate) mod init;
pub(crate) mod query;
pub(crate) mod receipt;
pub(crate) mod schema;
pub(crate) mod version;
