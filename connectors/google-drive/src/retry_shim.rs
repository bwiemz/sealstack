//! Drive-specific 403 discrimination.
//!
//! Drive's 403 responses cluster into three classes that need different
//! client-side handling. See spec §8 for the full taxonomy.

use std::time::Duration;

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::http::{HttpClient, HttpResponse};

/// Classification of a Drive 403 response.
#[derive(Debug)]
pub enum Drive403Action {
    /// Reserved for future use (explicit Retry-After on 403 if Drive emits it).
    WaitThenRetry(Duration),
    /// Short-term rate limit exceeded. Retry with exponential backoff
    /// (500ms × 2^attempt), up to 5 attempts. After budget exhaustion,
    /// surface as `RateLimited`.
    BackoffThenRetry,
    /// Daily quota exhausted. Retrying buys nothing until UTC midnight.
    /// Surface as `RateLimited` immediately.
    QuotaExhausted,
    /// Permission denied. Surface as `Backend` with comma-joined reasons.
    PermissionDenied {
        /// Comma-joined reason codes extracted from the Drive error body,
        /// e.g. `"domainPolicy,insufficientPermissions"`.
        reason: String,
    },
}

/// Classify a Drive 403 response body.
///
/// `_headers` is unused in v1 (Drive doesn't typically emit Retry-After on
/// 403); kept in signature for symmetry with github's shim.
#[must_use]
pub fn classify_drive_403(_headers: &[(String, String)], body: &str) -> Drive403Action {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let reasons: Vec<&str> = parsed
        .pointer("/error/errors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.get("reason").and_then(|r| r.as_str()))
                .collect()
        })
        .unwrap_or_default();

    // Daily quota wins over short-term rate-limit when both signals are present:
    // retrying a daily-quota-exhausted endpoint buys nothing.
    if reasons
        .iter()
        .any(|r| matches!(*r, "quotaExceeded" | "dailyLimitExceeded"))
    {
        return Drive403Action::QuotaExhausted;
    }
    if reasons
        .iter()
        .any(|r| matches!(*r, "userRateLimitExceeded" | "rateLimitExceeded"))
    {
        return Drive403Action::BackoffThenRetry;
    }
    // Comma-join all reasons for full diagnostic context. Operator-visible
    // message: "drive 403: permission denied (domainPolicy,insufficientPermissions)".
    let reason = if reasons.is_empty() {
        "(no reason in body)".to_owned()
    } else {
        reasons.join(",")
    };
    Drive403Action::PermissionDenied { reason }
}

/// At most 5 attempts: initial + 4 shim-guided retries.
///
/// Differs from `send_with_gh_shim`:
/// - Allows 4 retries (not 1) because Drive's per-user 10 req/sec limit
///   fires regularly; one retry isn't enough headroom.
/// - Distinguishes `QuotaExhausted` (immediate `RateLimited`) from
///   `BackoffThenRetry` (loop, then `RateLimited`).
pub async fn send_with_drive_shim<F>(
    http: &HttpClient,
    make_request: F,
) -> SealStackResult<HttpResponse>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        match http.send(make_request()).await {
            Ok(resp) => return Ok(resp),
            Err(SealStackError::HttpStatus {
                status: 403,
                headers,
                body,
            }) => {
                match classify_drive_403(&headers, &body) {
                    Drive403Action::BackoffThenRetry if attempt < 4 => {
                        // Backoff: 500ms, 1s, 2s, 4s. Cumulative ~7.5s.
                        // Drive's per-user rate-limit window is 100s; this
                        // schedule reaches roughly 7.5% of one window.
                        // Revisit if pilot telemetry shows budget-exhaustion
                        // at meaningful rates.
                        let delay = Duration::from_millis(500 * (1u64 << attempt));
                        // Demoted to debug — first attempts in a backoff loop
                        // are the system working as designed, not warnings.
                        // warn is reserved for budget exhaustion below.
                        tracing::debug!(
                            ?delay,
                            attempt,
                            "drive: 403 rate-limit, backing off before retry"
                        );
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                    }
                    Drive403Action::BackoffThenRetry => {
                        tracing::warn!(
                            attempts = 5,
                            "drive: 403 rate-limit retry budget exhausted"
                        );
                        return Err(SealStackError::RateLimited);
                    }
                    Drive403Action::QuotaExhausted => {
                        tracing::warn!(
                            "drive: daily quota exhausted, not retrying until UTC midnight"
                        );
                        return Err(SealStackError::RateLimited);
                    }
                    Drive403Action::WaitThenRetry(_) => {
                        // Reserved for future use; not produced by classify_drive_403 in v1.
                        return Err(SealStackError::Backend(
                            "drive: unexpected WaitThenRetry classification".to_owned(),
                        ));
                    }
                    Drive403Action::PermissionDenied { reason } => {
                        return Err(SealStackError::Backend(format!(
                            "drive 403: permission denied ({reason})"
                        )));
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
}
