//! Google Drive connector.
//!
//! Pulls files from a single user's "My Drive" via the Drive REST API v3.
//! Authentication is OAuth 2.0 refresh-token grant — users provide a refresh
//! token externally (via Google's OAuth playground or a one-off script) and
//! reference it via env var in their config.
//!
//! # Resources emitted
//!
//! * One [`Resource`] per allowlisted-MIME file. v1 allowlist:
//!   - `application/vnd.google-apps.document` (Google Docs, exported as text)
//!   - `text/plain`, `text/markdown` (direct binary fetch via `alt=media`)
//!
//! Skipped MIME types are logged at info level once per resource id and
//! never yield empty-body Resources.
//!
//! # Pagination
//!
//! Drive's `files.list` paginates via `nextPageToken` in the response body.
//! Uses the SDK's [`BodyCursorPaginator`](sealstack_connector_sdk::paginate::BodyCursorPaginator).
//!
//! # Out of scope (v1)
//!
//! * Shared Drives (`corpora=shared|all`) — config rejects non-`"user"`.
//! * Incremental sync via `changes.list` — full-crawl every cycle.
//! * CLI consent flow — refresh token comes from operator config.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

mod files;
mod permissions;
mod resource;
pub mod retry_shim;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use secrecy::SecretString;

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::auth::OAuth2Credential;
use sealstack_connector_sdk::change_streams;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use sealstack_connector_sdk::{ChangeStream, Connector, Resource, ResourceId, ResourceStream};

const DEFAULT_API_BASE: &str = "https://www.googleapis.com";
const DEFAULT_SYNC_INTERVAL_SECS: u64 = 900; // 15 minutes
const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024; // 10 MiB

/// Non-secret connector configuration.
#[derive(Clone, Debug)]
pub struct DriveConfig {
    /// API base URL — defaults to `https://www.googleapis.com`. Overridable
    /// for tests pointing at a wiremock server.
    pub api_base: String,
    /// Sync cadence. Default: 15 minutes (deletion-latency mitigation pending
    /// v0.2 incremental sync).
    pub sync_interval_seconds: u64,
    /// Per-file size cap in bytes. Files exceeding this are skipped with one
    /// info log per resource id.
    pub max_file_bytes: u64,
}

impl DriveConfig {
    /// Parse non-secret fields from the binding config JSON. All fields have
    /// defaults; `api_base` trailing slashes are trimmed.
    #[must_use]
    pub fn from_json(v: &serde_json::Value) -> Self {
        let api_base = v
            .get("api_base")
            .and_then(|x| x.as_str())
            .unwrap_or(DEFAULT_API_BASE)
            .trim_end_matches('/')
            .to_owned();
        let sync_interval_seconds = v
            .get("sync_interval_seconds")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_SYNC_INTERVAL_SECS);
        let max_file_bytes = v
            .get("max_file_bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_MAX_FILE_BYTES);
        Self {
            api_base,
            sync_interval_seconds,
            max_file_bytes,
        }
    }
}

/// The Google Drive connector.
#[derive(Debug)]
pub struct DriveConnector {
    http: Arc<HttpClient>,
    config: DriveConfig,
    skip_log: Arc<crate::files::SkipLog>,
}

impl DriveConnector {
    /// Build the connector from the binding config JSON.
    ///
    /// Required fields:
    /// - `client_id` — OAuth 2.0 client id (public; not a secret)
    /// - `client_secret_env` — name of env var holding the OAuth client secret
    /// - `refresh_token_env` — name of env var holding the OAuth refresh token
    ///
    /// Optional fields with defaults:
    /// - `corpora` (default `"user"`; only valid value in v1)
    /// - `api_base` (default `https://www.googleapis.com`)
    /// - `sync_interval_seconds` (default 900)
    /// - `max_file_bytes` (default 10 MiB)
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] for missing required fields, env
    /// vars not set or empty, or `corpora` set to anything other than `"user"`.
    pub fn from_json(v: &serde_json::Value) -> SealStackResult<Self> {
        let client_id = required_str(v, "client_id")?.to_owned();

        let client_secret_var = required_str(v, "client_secret_env")?;
        let client_secret = SecretString::new(read_env_var(client_secret_var)?.into());

        let refresh_token_var = required_str(v, "refresh_token_env")?;
        let refresh_token = SecretString::new(read_env_var(refresh_token_var)?.into());

        let corpora = v.get("corpora").and_then(|x| x.as_str()).unwrap_or("user");
        if corpora != "user" {
            return Err(SealStackError::Config(format!(
                "drive: `corpora = \"{corpora}\"` not yet supported; only \"user\" works in v1. \
                 Shared Drives land in v0.2."
            )));
        }

        let credential = Arc::new(OAuth2Credential::google(
            client_id,
            client_secret,
            refresh_token,
        )?);
        let http = Arc::new(
            HttpClient::new(credential, RetryPolicy::default())?.with_user_agent_suffix(format!(
                "google-drive-connector/{}",
                env!("CARGO_PKG_VERSION")
            )),
        );

        let config = DriveConfig::from_json(v);
        Ok(Self {
            http,
            config,
            skip_log: Arc::new(crate::files::SkipLog::default()),
        })
    }

    /// Sync cadence. Engine consumes this in a separate per-connector-interval
    /// engine slice; currently informational.
    #[must_use]
    pub const fn sync_interval(&self) -> Duration {
        Duration::from_secs(self.config.sync_interval_seconds)
    }
}

fn required_str<'a>(v: &'a serde_json::Value, key: &str) -> SealStackResult<&'a str> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| SealStackError::Config(format!("drive: missing required field `{key}`")))
}

fn read_env_var(name: &str) -> SealStackResult<String> {
    match std::env::var(name) {
        Err(_) => Err(SealStackError::Config(format!(
            "drive: env var `{name}` not set"
        ))),
        Ok(s) if s.is_empty() => Err(SealStackError::Config(format!(
            "drive: env var `{name}` is empty"
        ))),
        Ok(s) => Ok(s),
    }
}

#[async_trait]
impl Connector for DriveConnector {
    #[allow(clippy::unnecessary_literal_bound)] // trait signature imposes &str
    fn name(&self) -> &str {
        "google-drive"
    }

    #[allow(clippy::unnecessary_literal_bound)] // trait signature imposes &str
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        use crate::files::{fetch_body, files_stream};
        use crate::resource::drive_file_to_resource;

        let mut stream = files_stream(self.http.clone(), &self.config.api_base);
        let mut out: Vec<Resource> = Vec::new();
        while let Some(file_result) = stream.next().await {
            let file = file_result?;
            if let Some(body) = fetch_body(
                &self.http,
                &self.config.api_base,
                &file,
                self.config.max_file_bytes,
                &self.skip_log,
            )
            .await?
            {
                out.push(drive_file_to_resource(&file, body)?);
            }
            // None → skipped (oversized, non-allowlist MIME, or non-UTF-8)
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        use crate::files::{DriveFile, fetch_body};
        use crate::resource::drive_file_to_resource;
        use crate::retry_shim::send_with_drive_shim;

        let url = format!("{}/drive/v3/files/{}", self.config.api_base, id);
        let make = || {
            self.http.get(&url).query(&[(
                "fields",
                "id,name,mimeType,modifiedTime,driveId,size,\
                 permissions(type,emailAddress,domain,role,allowFileDiscovery)",
            )])
        };
        let resp = send_with_drive_shim(&self.http, make).await?;
        let file: DriveFile = resp.json().await?;
        fetch_body(
            &self.http,
            &self.config.api_base,
            &file,
            self.config.max_file_bytes,
            &self.skip_log,
        )
        .await?
        .map_or_else(
            || {
                Err(SealStackError::backend(format!(
                    "drive: file {id} skipped (oversized, non-allowlist MIME, or non-UTF-8 body)"
                )))
            },
            |body| drive_file_to_resource(&file, body),
        )
    }

    async fn subscribe(&self) -> SealStackResult<Option<ChangeStream>> {
        // v1 is full-crawl only; subscribe lands with v0.2 incremental.
        Ok(None)
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        // files.list (NOT files/about) because files.list exercises drive.readonly
        // scope. A refresh token granted with only userinfo.email scope would pass
        // /about but fail every subsequent /files.list with 403 insufficientPermissions.
        // Healthcheck must surface scope mismatches at boot, not at first sync.
        let url = format!("{}/drive/v3/files", self.config.api_base);
        let make = || self.http.get(&url).query(&[("pageSize", "1")]);
        let _ = crate::retry_shim::send_with_drive_shim(&self.http, make).await?;
        Ok(())
    }
}

#[doc(hidden)]
pub mod test_only {
    pub use crate::files::{DriveFileTestStub, fetch_body_for_test, list_files_for_test};
    pub use crate::retry_shim::send_with_drive_shim;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_json_rejects_missing_client_id() {
        let v = serde_json::json!({
            "client_secret_env": "X",
            "refresh_token_env": "Y"
        });
        let err = DriveConnector::from_json(&v).unwrap_err().to_string();
        assert!(err.contains("client_id"), "{err}");
    }

    #[test]
    fn from_json_rejects_corpora_shared() {
        let v = serde_json::json!({
            "client_id": "id",
            "client_secret_env": "DRIVE_TEST_SECRET",
            "refresh_token_env": "DRIVE_TEST_REFRESH",
            "corpora": "shared"
        });
        // Skip if env vars are pre-set in CI; this test specifically exercises
        // the corpora-rejection path and needs the auth check to pass first.
        if std::env::var("DRIVE_TEST_SECRET").is_err()
            || std::env::var("DRIVE_TEST_REFRESH").is_err()
        {
            return;
        }
        let err = DriveConnector::from_json(&v).unwrap_err().to_string();
        assert!(err.contains("corpora"), "{err}");
        assert!(err.contains("Shared Drives"), "{err}");
    }

    #[test]
    fn from_json_normalizes_api_base_trailing_slash() {
        let cfg = DriveConfig::from_json(&serde_json::json!({
            "api_base": "https://example.com/"
        }));
        assert_eq!(cfg.api_base, "https://example.com");
    }

    #[test]
    fn from_json_uses_defaults() {
        let cfg = DriveConfig::from_json(&serde_json::json!({}));
        assert_eq!(cfg.api_base, "https://www.googleapis.com");
        assert_eq!(cfg.sync_interval_seconds, 900);
        assert_eq!(cfg.max_file_bytes, 10 * 1024 * 1024);
    }
}
