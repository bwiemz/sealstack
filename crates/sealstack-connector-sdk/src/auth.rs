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
