//! HTTP client wrapper with auth injection, UA composition, and retry.
//!
//! The retry policy is baked in: every request through [`HttpClient`] goes
//! through the policy from [`super::retry::RetryPolicy`]. There is no
//! non-retrying request path in v1.

use std::sync::Arc;
use std::time::Instant;

use rand::Rng;
use sealstack_common::{SealStackError, SealStackResult};

use crate::auth::Credential;
use crate::retry::{parse_retry_after, RetryPolicy};

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

/// Wrapped HTTP response returned by [`HttpClient::send`].
///
/// The body-size cap is enforced by the `bytes` / `json` accessors (Task 8).
#[derive(Debug)]
pub struct HttpResponse {
    inner: reqwest::Response,
    body_cap_bytes: usize,
}

impl HttpResponse {
    /// HTTP status code.
    #[must_use]
    pub fn status(&self) -> reqwest::StatusCode {
        self.inner.status()
    }

    /// Access a response header value.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.inner
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
    }

    /// Consume the response and yield the underlying `reqwest::Response`.
    ///
    /// Escape hatch for callers that want full access before the body-cap
    /// machinery lands.
    #[must_use]
    pub fn into_inner(self) -> reqwest::Response {
        self.inner
    }

    /// Expose the body cap for downstream body-reading helpers.
    #[allow(dead_code)]
    pub(crate) const fn body_cap_bytes(&self) -> usize {
        self.body_cap_bytes
    }
}

impl HttpClient {
    /// Execute a request under the retry policy.
    ///
    /// Injects the `Authorization` header from the configured [`Credential`]
    /// and the client's `User-Agent`. Applies retry logic per the policy.
    /// See the spec §6 for the retry-decision table.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Backend`] for non-retryable 4xx responses
    /// and for credential build failures. Returns
    /// [`SealStackError::RetryExhausted`] when the retry budget is consumed
    /// without a success.
    pub async fn send(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> SealStackResult<HttpResponse> {
        let start = Instant::now();
        let mut attempt: u32 = 0;
        // Sentinel: always overwritten before read; rustc cannot see that the
        // loop body sets `last_err` before any `break`.
        #[allow(unused_assignments)]
        let mut last_err: SealStackError = SealStackError::backend("unknown");

        loop {
            let try_rb = rb
                .try_clone()
                .ok_or_else(|| SealStackError::backend("request body not cloneable"))?;
            let auth = self.credential.authorization_header().await?;
            let req = try_rb
                .header("Authorization", auth)
                .header("User-Agent", &self.user_agent);

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(HttpResponse {
                            inner: resp,
                            body_cap_bytes: self.body_cap_bytes,
                        });
                    }
                    attempt += 1;
                    if status.is_client_error()
                        && status != reqwest::StatusCode::REQUEST_TIMEOUT
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        return Err(SealStackError::Backend(format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        )));
                    }
                    let delay = retry_delay_for(
                        &self.retry,
                        attempt - 1,
                        resp.headers()
                            .get("Retry-After")
                            .and_then(|v| v.to_str().ok()),
                    );
                    last_err = SealStackError::Backend(format!(
                        "HTTP {} (attempt {attempt})",
                        status.as_u16()
                    ));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    attempt += 1;
                    last_err = SealStackError::backend(format!("network: {e}"));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    let delay = retry_delay_for(&self.retry, attempt - 1, None);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(SealStackError::RetryExhausted {
            attempts: attempt,
            total_duration: start.elapsed(),
            last_error: Box::new(last_err),
        })
    }
}

/// Compute the next sleep duration.
///
/// - If `retry_after` is present and parseable, use `min(max_delay,
///   retry_after + rand(0..1000ms))`.
/// - Otherwise exponential: `delay = min(max_delay, base * 2^attempt)`, then
///   full-jitter with `rand(0..delay)`.
fn retry_delay_for(
    policy: &RetryPolicy,
    attempt: u32,
    retry_after_header: Option<&str>,
) -> std::time::Duration {
    use std::time::Duration;

    if let Some(raw) = retry_after_header
        && let Some(base) = parse_retry_after(raw)
    {
        let jitter_ms = rand::thread_rng().gen_range(0..1000);
        let with_jitter = base + Duration::from_millis(jitter_ms);
        return std::cmp::min(policy.max_delay, with_jitter);
    }

    let shift = attempt.min(20); // cap at 2^20 to avoid overflow
    let exp = policy
        .base_delay
        .saturating_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX));
    let capped = std::cmp::min(policy.max_delay, exp);
    // SAFETY: `capped` is bounded by `policy.max_delay` which is a Duration
    // configured by the caller. A delay above 2^63 ms (~300 million years)
    // is structurally impossible in practice; the cast from u128 to u64 is safe.
    #[allow(clippy::cast_possible_truncation)]
    let jittered_ms = rand::thread_rng().gen_range(0..=capped.as_millis() as u64);
    Duration::from_millis(jittered_ms)
}

#[cfg(test)]
mod retry_delay_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn respects_max_delay_cap() {
        let p = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
        };
        for _ in 0..20 {
            let d = retry_delay_for(&p, 30, None);
            assert!(d <= p.max_delay);
        }
    }

    #[test]
    fn retry_after_dominates_exponential() {
        let p = RetryPolicy::default();
        let d = retry_delay_for(&p, 0, Some("2"));
        assert!(d >= Duration::from_secs(2));
        assert!(d < Duration::from_secs(4)); // 2s + <1s jitter
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
