//! HTTP client wrapper with auth injection, UA composition, and retry.
//!
//! The retry policy is baked in: every request through [`HttpClient`] goes
//! through the policy from [`super::retry::RetryPolicy`]. There is no
//! non-retrying request path in v1.

use std::sync::Arc;

use sealstack_common::{SealStackError, SealStackResult};

use crate::auth::Credential;
use crate::retry::RetryPolicy;

/// Hard upper bound on the response body-size cap, in bytes (500 MiB).
///
/// Configuring [`HttpClient::with_body_cap`] above this is a configuration
/// error — misconfiguration cannot disable `DoS` protection entirely.
pub const MAX_BODY_CAP_BYTES: usize = 500 * 1024 * 1024;

/// Default response body-size cap, in bytes (50 MiB).
pub const DEFAULT_BODY_CAP_BYTES: usize = 50 * 1024 * 1024;

/// Connector-side HTTP client.
pub struct HttpClient {
    inner: reqwest::Client,
    #[allow(dead_code)]
    credential: Arc<dyn Credential>,
    retry: RetryPolicy,
    user_agent: String,
    body_cap_bytes: usize,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("credential", &"<dyn Credential>")
            .field("retry", &self.retry)
            .field("user_agent", &self.user_agent)
            .field("body_cap_bytes", &self.body_cap_bytes)
            .finish_non_exhaustive()
    }
}

impl HttpClient {
    /// Base User-Agent without suffix.
    fn base_ua() -> String {
        format!("sealstack-connector-sdk/{} (rust)", env!("CARGO_PKG_VERSION"))
    }

    /// Build a client with the given credential and retry policy.
    pub fn new(
        credential: Arc<dyn Credential>,
        retry: RetryPolicy,
    ) -> SealStackResult<Self> {
        let inner = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| SealStackError::backend(format!("reqwest client build: {e}")))?;
        Ok(Self {
            inner,
            credential,
            retry,
            user_agent: Self::base_ua(),
            body_cap_bytes: DEFAULT_BODY_CAP_BYTES,
        })
    }

    /// Append a connector-identifying suffix to the User-Agent.
    ///
    /// Produces e.g. `sealstack-connector-sdk/1.0.0 (rust) github-connector/0.1.0`.
    #[must_use]
    pub fn with_user_agent_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.user_agent = format!("{} {}", Self::base_ua(), suffix.into());
        self
    }

    /// Configure the response body-size cap.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] if `cap_bytes` exceeds
    /// [`MAX_BODY_CAP_BYTES`].
    pub fn with_body_cap(mut self, cap_bytes: usize) -> SealStackResult<Self> {
        if cap_bytes > MAX_BODY_CAP_BYTES {
            return Err(SealStackError::Config(format!(
                "body cap {cap_bytes} exceeds hard maximum {MAX_BODY_CAP_BYTES}"
            )));
        }
        self.body_cap_bytes = cap_bytes;
        Ok(self)
    }

    /// Returns the composed User-Agent string (for tests + diagnostics).
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Returns the current body cap, in bytes (for tests + diagnostics).
    #[must_use]
    pub const fn body_cap_bytes(&self) -> usize {
        self.body_cap_bytes
    }

    /// Begin a GET request.
    ///
    /// Callers chain `.query()`, `.header()`, etc. and then pass to
    /// [`HttpClient::send`] to execute under the retry policy.
    pub fn get(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.inner.get(url)
    }

    /// Begin a POST request.
    pub fn post(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.inner.post(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::StaticToken;

    fn test_client() -> HttpClient {
        HttpClient::new(
            Arc::new(StaticToken::new("t")),
            RetryPolicy::default(),
        )
        .unwrap()
    }

    #[test]
    fn user_agent_has_expected_shape() {
        let c = test_client();
        let ua = c.user_agent();
        assert!(ua.starts_with("sealstack-connector-sdk/"));
        assert!(ua.contains("(rust)"));
    }

    #[test]
    fn user_agent_suffix_appended() {
        let c = test_client().with_user_agent_suffix("github-connector/0.1.0");
        assert!(c.user_agent().ends_with(" github-connector/0.1.0"));
    }

    #[test]
    fn body_cap_rejects_above_hard_max() {
        let c = test_client();
        let too_big = MAX_BODY_CAP_BYTES + 1;
        let err = c.with_body_cap(too_big).unwrap_err().to_string();
        assert!(err.contains("hard maximum"), "{err}");
    }

    #[test]
    fn body_cap_default_is_50_mib() {
        let c = test_client();
        assert_eq!(c.body_cap_bytes(), 50 * 1024 * 1024);
    }
}
