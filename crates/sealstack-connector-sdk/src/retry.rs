//! Retry policy for [`super::http::HttpClient`].
//!
//! Exports the [`RetryPolicy`] configuration type and the [`parse_retry_after`]
//! helper. The retry loop itself lives in `http.rs` because it is tightly
//! integrated with request sending.

use std::time::Duration;

/// Reactive retry policy.
///
/// Applied by `HttpClient` to every outbound request. See the spec §6 for the
/// backoff schedule and the 401 invalidate-retry rule.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Maximum total attempts (1 initial + up to `max_attempts - 1` retries).
    pub max_attempts: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Cap on any single sleep between attempts.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Parse a `Retry-After` header value.
///
/// v1 supports integer seconds only (the form used by GitHub, Slack, Stripe,
/// and the vast majority of servers). HTTP-date form (`"Wed, 21 Oct 2015
/// 07:28:00 GMT"`) returns `None`, and the caller falls back to exponential
/// backoff — acceptable because servers that send HTTP-date also send
/// integer seconds in sibling headers, and exponential backoff is a safe
/// fallback. HTTP-date support is a follow-up if observed need warrants it.
///
/// Returns `None` for negative or unparseable values.
#[must_use]
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let trimmed = value.trim();
    let secs = trimmed.parse::<i64>().ok()?;
    if secs < 0 {
        return None;
    }
    // SAFETY: We have validated that secs >= 0, so the cast is safe.
    #[allow(clippy::cast_sign_loss)]
    Some(Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_retry_after_integer_seconds() {
        assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after(" 0 "), Some(Duration::from_secs(0)));
    }

    #[test]
    fn parse_retry_after_rejects_negative() {
        assert_eq!(parse_retry_after("-1"), None);
    }

    #[test]
    fn parse_retry_after_rejects_garbage() {
        assert_eq!(parse_retry_after("soon"), None);
    }

    #[test]
    fn parse_retry_after_http_date_returns_none_in_v1() {
        // v1 supports integer seconds only; HTTP-date falls through to
        // exponential backoff rather than being parsed.
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2099 07:28:00 GMT"),
            None,
        );
    }

    #[test]
    fn default_policy_matches_spec() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 5);
        assert_eq!(p.base_delay, Duration::from_millis(500));
        assert_eq!(p.max_delay, Duration::from_secs(30));
    }
}
