//! Authentication primitives for connectors.
//!
//! v1 ships `StaticToken` (PATs, bot tokens, API keys). Future modules add
//! OAuth 2.0 authorization-code + refresh, Google service-account JWTs, etc.,
//! each as an additional [`Credential`] implementation.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use sealstack_common::{SealStackError, SealStackResult};

/// Source of the `Authorization` header value for an outbound request.
///
/// # Contract
///
/// `authorization_header` returns the full header value including the scheme
/// prefix (e.g. `"Bearer abc123"`). v1 implementations always use the bearer
/// scheme; future non-bearer schemes (Basic, HMAC-signed) add new
/// implementations without changing the trait.
///
/// Per-request allocation is intentional: OAuth's lock-guarded refresh path
/// requires owned `String` values to cross `.await` safely. The ~50-byte
/// clone is dominated by HTTP transport costs.
///
/// Caching implementations (e.g. OAuth) must use async-aware synchronization
/// primitives (`tokio::sync::Mutex`, `arc-swap`, etc.) to avoid holding
/// locks across `.await` points.
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Returns the full `Authorization` header value, including scheme prefix.
    async fn authorization_header(&self) -> SealStackResult<String>;

    /// Invalidate any cached credential material.
    ///
    /// Called by `HttpClient` before retrying a 401. Default is a no-op for
    /// credentials that cannot refresh (e.g. [`StaticToken`]).
    async fn invalidate(&self) {}
}

/// Long-lived static bearer token (PAT, bot token, API key).
///
/// Token material is held in a [`SecretString`] so it zeroes on drop and
/// cannot be accidentally printed via `Debug`.
pub struct StaticToken(SecretString);

impl StaticToken {
    /// Build from any string value.
    pub fn new(token: impl Into<String>) -> Self {
        Self(SecretString::from(token.into()))
    }

    /// Read a token from an environment variable.
    ///
    /// Distinguishes "variable not set" from "variable set to empty string"
    /// with two distinct error messages — both surface as
    /// [`SealStackError::Config`].
    pub fn from_env(var: &str) -> SealStackResult<Self> {
        Self::from_env_result(var, std::env::var(var))
    }

    /// Pure helper that classifies the result of an environment lookup.
    ///
    /// Separated out for testability: exercising both error branches
    /// directly avoids the process-level env manipulation that would
    /// otherwise require `unsafe { std::env::set_var }` (not permitted
    /// under the crate's `forbid(unsafe_code)` lint).
    fn from_env_result(
        var: &str,
        result: Result<String, std::env::VarError>,
    ) -> SealStackResult<Self> {
        match result {
            Err(_) => Err(SealStackError::Config(format!("env var `{var}` not set"))),
            Ok(s) if s.is_empty() => {
                Err(SealStackError::Config(format!("env var `{var}` is empty")))
            }
            Ok(s) => Ok(Self::new(s)),
        }
    }
}

impl std::fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StaticToken").field(&"<redacted>").finish()
    }
}

#[async_trait]
impl Credential for StaticToken {
    async fn authorization_header(&self) -> SealStackResult<String> {
        Ok(format!("Bearer {}", self.0.expose_secret()))
    }
    // invalidate() default no-op preserves v1 "same token → same 401 →
    // Unauthorized" semantic.
}

use std::time::{Duration, Instant};

/// OAuth 2.0 credential using the refresh-token grant.
///
/// Caches the access token in-memory with a 60-second skew margin and
/// coalesces concurrent refresh attempts via a `tokio::sync::Mutex`.
/// Transient refresh failures (5xx, network) are negative-cached for 5
/// seconds to prevent serialized-retry stampedes; permanent failures
/// (`invalid_grant`, `invalid_client`, `invalid_scope`) are not cached
/// because retrying just delays the inevitable error.
///
/// # See also
///
/// Planned `microsoft(tenant_id, ...)` and `notion(...)` convenience
/// constructors as those providers come online. The pattern is hardcoding
/// well-known token endpoints while [`Self::new`] stays generic.
pub struct OAuth2Credential {
    client_id: String,
    client_secret: SecretString,
    refresh_token: SecretString,
    token_endpoint: String,
    pub(crate) cache: tokio::sync::Mutex<CachedAccess>,
    inner: reqwest::Client,
}

#[derive(Default)]
pub(crate) struct CachedAccess {
    pub(crate) access_token: Option<SecretString>,
    pub(crate) valid_until: Option<Instant>,
    pub(crate) negative_cache: Option<NegativeCache>,
}

pub(crate) struct NegativeCache {
    expires: Instant,
    message: String,
}

const REFRESH_SKEW_SECS: u64 = 60;
const NEGATIVE_CACHE_SECS: u64 = 5;

impl OAuth2Credential {
    /// Construct against an arbitrary OAuth 2.0 token endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Backend`] if the underlying `reqwest::Client`
    /// fails to build (e.g., TLS configuration issue).
    pub fn new(
        client_id: impl Into<String>,
        client_secret: SecretString,
        refresh_token: SecretString,
        token_endpoint: impl Into<String>,
    ) -> SealStackResult<Self> {
        let inner = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| SealStackError::backend(format!("oauth2 client build: {e}")))?;
        Ok(Self {
            client_id: client_id.into(),
            client_secret,
            refresh_token,
            token_endpoint: token_endpoint.into(),
            cache: tokio::sync::Mutex::new(CachedAccess::default()),
            inner,
        })
    }

    /// Convenience constructor for Google's well-known token endpoint.
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`].
    pub fn google(
        client_id: impl Into<String>,
        client_secret: SecretString,
        refresh_token: SecretString,
    ) -> SealStackResult<Self> {
        Self::new(
            client_id,
            client_secret,
            refresh_token,
            "https://oauth2.googleapis.com/token",
        )
    }

    /// Send the form-encoded refresh request and parse the response.
    ///
    /// Light retry (3 attempts, 200/400/800ms exponential) on 5xx + network
    /// errors. Hand-rolled rather than going through `HttpClient` because
    /// `HttpClient::send` calls `Credential::authorization_header`, which
    /// would be circular. This is the only place in the SDK that hand-rolls
    /// retry; consolidate if a second OAuth credential type appears with
    /// different retry semantics.
    async fn refresh_with_retry(&self) -> SealStackResult<(String, u64)> {
        let mut last_err = SealStackError::backend("unknown");
        for attempt in 0..3u32 {
            match self.refresh_once().await {
                Ok((tok, expires_in)) => return Ok((tok, expires_in)),
                Err(e @ (SealStackError::Unauthorized(_) | SealStackError::Config(_))) => {
                    return Err(e);
                }
                Err(e) => {
                    last_err = e;
                    if attempt < 2 {
                        let delay_ms = 200 * (1u64 << attempt);
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    async fn refresh_once(&self) -> SealStackResult<(String, u64)> {
        #[derive(serde::Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        #[derive(serde::Deserialize)]
        struct ErrorResponse {
            error: String,
            #[serde(default)]
            #[allow(dead_code)]
            error_description: Option<String>,
        }

        let resp = self
            .inner
            .post(&self.token_endpoint)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.expose_secret()),
                ("refresh_token", self.refresh_token.expose_secret()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| SealStackError::backend(format!("oauth2 transport: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| SealStackError::backend(format!("oauth2 body read: {e}")))?;

        if status.is_success() {
            let tr: TokenResponse = serde_json::from_str(&body)
                .map_err(|e| SealStackError::backend(format!("oauth2 parse: {e}")))?;
            return Ok((tr.access_token, tr.expires_in));
        }
        if status.as_u16() == 400 {
            let er: ErrorResponse = serde_json::from_str(&body).unwrap_or_else(|_| ErrorResponse {
                error: "unknown".to_owned(),
                error_description: None,
            });
            match er.error.as_str() {
                "invalid_grant" => {
                    return Err(SealStackError::Unauthorized(
                        "OAuth2 refresh failed: invalid_grant".to_owned(),
                    ));
                }
                "invalid_client" | "invalid_scope" => {
                    return Err(SealStackError::Config(format!(
                        "OAuth2 misconfiguration: {}",
                        er.error
                    )));
                }
                other => {
                    return Err(SealStackError::Backend(format!(
                        "OAuth2 refresh failed: {other}"
                    )));
                }
            }
        }
        Err(SealStackError::Backend(format!(
            "OAuth2 refresh failed: HTTP {status}"
        )))
    }
}

impl std::fmt::Debug for OAuth2Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Credential")
            .field("client_id", &self.client_id)
            .field("token_endpoint", &self.token_endpoint)
            .field("client_secret", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Credential for OAuth2Credential {
    async fn authorization_header(&self) -> SealStackResult<String> {
        let mut cache = self.cache.lock().await;

        // Negative cache hit (refresh failed recently within 5s window).
        if let Some(neg) = &cache.negative_cache {
            if neg.expires > Instant::now() {
                return Err(SealStackError::Backend(neg.message.clone()));
            }
            cache.negative_cache = None; // stale; clear and proceed
        }

        // Positive cache hit. 60-second margin absorbs server-side
        // validator-cache latency on Google's edge — Google may treat
        // tokens as expired slightly before the reported expires_in due
        // to refresh latency in their token validators. Not network RTT
        // (sub-second) or NTP drift (sub-second).
        if let (Some(tok), Some(until)) = (&cache.access_token, &cache.valid_until)
            && Instant::now() + Duration::from_secs(REFRESH_SKEW_SECS) < *until
        {
            return Ok(format!("Bearer {}", tok.expose_secret()));
        }

        // Refresh.
        match self.refresh_with_retry().await {
            Ok((new_tok, expires_in)) => {
                let header = format!("Bearer {new_tok}");
                cache.access_token = Some(SecretString::new(new_tok.into()));
                cache.valid_until = Some(Instant::now() + Duration::from_secs(expires_in));
                Ok(header)
            }
            Err(e @ (SealStackError::Unauthorized(_) | SealStackError::Config(_))) => {
                // Permanent failures: no negative cache. Caching them just
                // delays the inevitable error.
                Err(e)
            }
            Err(e) => {
                // Transient failures (5xx, network, retry-budget-exhausted):
                // negative-cache for 5s to coalesce stampede.
                let message = format!("OAuth2 refresh failed: {e}");
                cache.negative_cache = Some(NegativeCache {
                    expires: Instant::now() + Duration::from_secs(NEGATIVE_CACHE_SECS),
                    message: message.clone(),
                });
                drop(cache);
                Err(SealStackError::Backend(message))
            }
        }
    }

    async fn invalidate(&self) {
        let mut cache = self.cache.lock().await;
        cache.access_token = None;
        cache.valid_until = None;
        // Note: negative_cache is NOT cleared on invalidate. If a refresh
        // just failed transiently, the next caller still gets fast-fail
        // for the cached window.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_token_emits_bearer_header() {
        let t = StaticToken::new("abc123");
        assert_eq!(t.authorization_header().await.unwrap(), "Bearer abc123");
    }

    #[tokio::test]
    async fn invalidate_is_noop_by_default() {
        let t = StaticToken::new("abc123");
        t.invalidate().await;
        assert_eq!(t.authorization_header().await.unwrap(), "Bearer abc123");
    }

    #[test]
    fn debug_redacts_secret() {
        let t = StaticToken::new("super-secret-value");
        let s = format!("{t:?}");
        assert!(!s.contains("super-secret-value"), "Debug leaked: {s}");
        assert!(s.contains("StaticToken"));
    }

    #[test]
    fn from_env_reports_missing_distinctly_from_empty() {
        // Exercise the classification helper directly. We avoid
        // `std::env::set_var` because Rust 2024 marked it unsafe, and the
        // crate forbids unsafe. The pure helper is equivalent.

        let err_missing =
            StaticToken::from_env_result("SEALSTACK_NOT_SET", Err(std::env::VarError::NotPresent))
                .unwrap_err()
                .to_string();
        assert!(
            err_missing.contains("not set"),
            "missing case: {err_missing}"
        );

        let err_empty = StaticToken::from_env_result("SEALSTACK_EMPTY", Ok(String::new()))
            .unwrap_err()
            .to_string();
        assert!(err_empty.contains("is empty"), "empty case: {err_empty}");

        // Valid value: helper returns Ok.
        assert!(StaticToken::from_env_result("SEALSTACK_OK", Ok("abc".to_owned())).is_ok());
    }
}

#[cfg(test)]
mod oauth2_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_credential(token_endpoint: &str) -> OAuth2Credential {
        OAuth2Credential::new(
            "client-id-123".to_owned(),
            SecretString::new("client-secret".to_owned().into()),
            SecretString::new("refresh-token".to_owned().into()),
            token_endpoint.to_owned(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn oauth2_caches_access_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.first",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .expect(1) // exactly one refresh — second call must hit cache.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let h1 = cred.authorization_header().await.unwrap();
        let h2 = cred.authorization_header().await.unwrap();
        assert_eq!(h1, "Bearer ya29.first");
        assert_eq!(h2, "Bearer ya29.first");
    }

    #[tokio::test]
    async fn oauth2_refreshes_after_expiry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.short",
                "expires_in": 1,
                "token_type": "Bearer"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.fresh",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let h1 = cred.authorization_header().await.unwrap();
        assert_eq!(h1, "Bearer ya29.short");
        // Force expiry by invalidating instead of sleeping past 60s skew + 1s.
        cred.invalidate().await;
        let h2 = cred.authorization_header().await.unwrap();
        assert_eq!(h2, "Bearer ya29.fresh");
    }

    #[tokio::test]
    async fn oauth2_invalidate_clears_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.first",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.second",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        assert_eq!(
            cred.authorization_header().await.unwrap(),
            "Bearer ya29.first"
        );
        cred.invalidate().await;
        assert_eq!(
            cred.authorization_header().await.unwrap(),
            "Bearer ya29.second"
        );
    }

    #[tokio::test]
    async fn oauth2_invalid_grant_returns_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Token has been expired or revoked."
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let err = cred.authorization_header().await.unwrap_err();
        match err {
            SealStackError::Unauthorized(msg) => {
                assert!(msg.contains("invalid_grant"), "{msg}");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oauth2_invalid_client_returns_config_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Client authentication failed."
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let err = cred.authorization_header().await.unwrap_err();
        match err {
            SealStackError::Config(msg) => {
                assert!(msg.contains("invalid_client"), "{msg}");
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oauth2_concurrent_refresh_coalesces() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.coalesced",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .expect(1) // refresh-coalescing: 5 concurrent calls → 1 refresh.
            .mount(&server)
            .await;

        let cred = Arc::new(make_credential(&format!("{}/token", server.uri())));
        let mut handles = Vec::new();
        for _ in 0..5 {
            let c = cred.clone();
            handles.push(tokio::spawn(async move {
                c.authorization_header().await.unwrap()
            }));
        }
        for h in handles {
            assert_eq!(h.await.unwrap(), "Bearer ya29.coalesced");
        }
    }

    #[tokio::test]
    async fn oauth2_negative_cache_coalesces_transient_failures() {
        let server = MockServer::start().await;
        // Fail every refresh attempt with 503. The hand-rolled retry inside
        // refresh_with_retry tries 3 times; second call hits negative cache.
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3) // 3 retries inside the first call; second call no hits.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let err1 = cred.authorization_header().await.unwrap_err();
        assert!(matches!(err1, SealStackError::Backend(_)));
        let err2 = cred.authorization_header().await.unwrap_err();
        assert!(matches!(err2, SealStackError::Backend(_)));
    }

    #[tokio::test]
    async fn oauth2_permanent_failures_do_not_negative_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant"
            })))
            .expect(2) // both calls hit the endpoint; permanent failures don't cache.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let _ = cred.authorization_header().await;
        let _ = cred.authorization_header().await;
    }

    #[tokio::test]
    async fn oauth2_negative_cache_clears_after_window() {
        // Verify the 5-second negative-cache window expires correctly: a
        // transient failure populates the cache, subsequent calls hit it,
        // and after the window passes the next call attempts a fresh refresh.
        //
        // Manipulate the cache directly rather than waiting wall-clock 5s —
        // we set negative_cache.expires to a past Instant, which simulates
        // window expiry without slowing the test.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.fresh_after_window",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .expect(1) // exactly one call — only fires after we mark the cache stale.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));

        // Pre-populate negative cache with an entry that has already expired.
        {
            let mut cache = cred.cache.lock().await;
            cache.negative_cache = Some(NegativeCache {
                expires: Instant::now()
                    .checked_sub(Duration::from_secs(1))
                    .expect("test clock supports 1s rewind"),
                message: "stale negative cache".to_owned(),
            });
        }

        // Call should bypass the stale negative-cache entry, refresh, succeed.
        let h = cred.authorization_header().await.unwrap();
        assert_eq!(h, "Bearer ya29.fresh_after_window");

        // After successful refresh, the negative cache should be cleared.
        let negative_cache_cleared = {
            let cache = cred.cache.lock().await;
            cache.negative_cache.is_none()
        };
        assert!(
            negative_cache_cleared,
            "stale negative cache should have been cleared"
        );
    }

    #[tokio::test]
    async fn oauth2_skew_triggers_refresh_at_60s_before_expiry() {
        // Pre-populate cache with a token that "expires" 50s from now.
        // 50s < 60s skew margin, so authorization_header should refresh.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.fresh",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        // Pre-populate via direct cache access.
        {
            let mut cache = cred.cache.lock().await;
            cache.access_token = Some(SecretString::new("ya29.almost_expired".into()));
            cache.valid_until = Some(std::time::Instant::now() + Duration::from_secs(50));
        }
        let h = cred.authorization_header().await.unwrap();
        assert_eq!(h, "Bearer ya29.fresh", "should have refreshed due to skew");
    }

    #[tokio::test]
    async fn oauth2_google_constructor_uses_correct_endpoint() {
        let cred = OAuth2Credential::google(
            "id".to_owned(),
            SecretString::new("secret".into()),
            SecretString::new("refresh".into()),
        )
        .unwrap();
        let dbg = format!("{cred:?}");
        assert!(
            dbg.contains("oauth2.googleapis.com/token"),
            "Debug should show token endpoint: {dbg}"
        );
    }

    #[test]
    fn oauth2_debug_redacts_secrets() {
        let cred = OAuth2Credential::new(
            "client-id-123".to_owned(),
            SecretString::new("super-secret-value".into()),
            SecretString::new("refresh-token-xyz".into()),
            "https://example.com/token".to_owned(),
        )
        .unwrap();
        let dbg = format!("{cred:?}");
        assert!(
            !dbg.contains("super-secret-value"),
            "client_secret leaked: {dbg}"
        );
        assert!(
            !dbg.contains("refresh-token-xyz"),
            "refresh_token leaked: {dbg}"
        );
        assert!(
            dbg.contains("client-id-123"),
            "client_id should be visible: {dbg}"
        );
        assert!(
            dbg.contains("example.com/token"),
            "endpoint should be visible: {dbg}"
        );
    }
}
