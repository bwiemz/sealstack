use std::time::Duration;

use sealstack_connector_github::retry_shim::{Github403Action, classify_github_403};

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn primary_rate_limit_waits_until_reset() {
    let reset = (time::OffsetDateTime::now_utc().unix_timestamp() + 60).to_string();
    let headers = pairs(&[
        ("X-RateLimit-Remaining", "0"),
        ("X-RateLimit-Reset", reset.as_str()),
    ]);
    match classify_github_403(&headers, "") {
        Github403Action::WaitThenRetry(d) => {
            assert!(
                d > Duration::from_secs(55) && d < Duration::from_secs(65),
                "{d:?}"
            );
        }
        other => panic!("expected WaitThenRetry, got {other:?}"),
    }
}

#[test]
fn secondary_rate_limit_retry_after_honored() {
    let headers = pairs(&[("Retry-After", "15")]);
    match classify_github_403(&headers, "") {
        Github403Action::WaitThenRetry(d) => {
            assert_eq!(d, Duration::from_secs(15));
        }
        other => panic!("expected WaitThenRetry, got {other:?}"),
    }
}

#[test]
fn secondary_rate_limit_body_marker_uses_backoff() {
    let body = r#"{"message":"You have exceeded a secondary rate limit."}"#;
    match classify_github_403(&[], body) {
        Github403Action::BackoffThenRetry => {}
        other => panic!("expected BackoffThenRetry, got {other:?}"),
    }
}

#[test]
fn plain_403_is_permission_denied() {
    match classify_github_403(&[], r#"{"message":"Resource not accessible"}"#) {
        Github403Action::PermissionDenied => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}
