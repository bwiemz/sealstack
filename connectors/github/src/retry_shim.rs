//! GitHub-specific 403 discrimination.
//!
//! GitHub's REST API emits three distinct 403 patterns that require
//! different client-side handling. See the design spec at
//! `docs/superpowers/specs/2026-04-24-connector-sdk-hardening-design.md` §8.

use std::time::Duration;

/// Classification of a GitHub 403 response.
#[derive(Debug)]
pub enum Github403Action {
    /// Case (a) primary rate limit or (b1) explicit `Retry-After`.
    ///
    /// The duration is how long to wait before a single retry.
    WaitThenRetry(Duration),
    /// Case (b2) secondary rate limit without explicit `Retry-After`.
    ///
    /// Caller applies its own exponential backoff (typically ~500ms to 1s).
    BackoffThenRetry,
    /// Case (c) real permission failure — do not retry.
    PermissionDenied,
}

/// Classify a GitHub 403 response.
///
/// `headers` are `(name, value)` pairs as surfaced by
/// `SealStackError::HttpStatus`. `body` is the response body as UTF-8 text
/// (already size-capped by `HttpClient`).
#[must_use]
pub fn classify_github_403(headers: &[(String, String)], body: &str) -> Github403Action {
    // Case (a): primary rate limit.
    if header_eq(headers, "X-RateLimit-Remaining", "0")
        && let Some(reset_unix) =
            header_value(headers, "X-RateLimit-Reset").and_then(|s| s.parse::<i64>().ok())
    {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // One extra second of slack for clock skew between client and server.
        let delta = reset_unix.saturating_sub(now).max(0).cast_unsigned();
        return Github403Action::WaitThenRetry(Duration::from_secs(delta + 1));
    }
    // Case (b1): explicit Retry-After.
    if let Some(secs) = header_value(headers, "Retry-After").and_then(|s| s.parse::<u64>().ok()) {
        return Github403Action::WaitThenRetry(Duration::from_secs(secs));
    }
    // Case (b2): body marker for secondary rate limit.
    if body.to_ascii_lowercase().contains("secondary rate limit") {
        return Github403Action::BackoffThenRetry;
    }
    // Case (c): everything else is permission-denied.
    Github403Action::PermissionDenied
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn header_eq(headers: &[(String, String)], name: &str, value: &str) -> bool {
    header_value(headers, name) == Some(value)
}
