//! End-to-end tests for the GitHub 403 shim via wiremock.
//!
//! The classifier itself is tested in `retry_shim.rs`; these tests exercise
//! the full path: `healthcheck` → `get_json` → `send_with_gh_shim` →
//! `HttpClient::send` returning `HttpStatus { status: 403, .. }` → match-arm
//! dispatch.

use sealstack_connector_github::GithubConnector;
use sealstack_connector_sdk::Connector;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(api_base: &str) -> serde_json::Value {
    serde_json::json!({
        "token": "ghp_test_token",
        "api_base": api_base,
    })
}

#[tokio::test]
async fn primary_rate_limit_triggers_retry_and_succeeds() {
    let server = MockServer::start().await;
    // Reset 1 second in the future → WaitThenRetry(~2s with slack).
    let reset = (time::OffsetDateTime::now_utc().unix_timestamp() + 1).to_string();
    // First call: 403 with primary rate-limit headers.
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(403)
                .append_header("X-RateLimit-Remaining", "0")
                .append_header("X-RateLimit-Reset", reset.as_str()),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Second call: 200 — healthcheck succeeds.
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"login": "octocat"})),
        )
        .mount(&server)
        .await;

    let cfg = config(&server.uri());
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    // The shim should wait ~2s (1s remaining + 1s slack) then retry.
    conn.healthcheck()
        .await
        .expect("healthcheck should succeed after rate-limit retry");
}

#[tokio::test]
async fn secondary_rate_limit_with_retry_after_succeeds() {
    let server = MockServer::start().await;
    // Retry-After: 0 → WaitThenRetry(0s) — near-instant.
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(403).append_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"login": "octocat"})),
        )
        .mount(&server)
        .await;

    let cfg = config(&server.uri());
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    conn.healthcheck()
        .await
        .expect("healthcheck should succeed after Retry-After honored");
}

#[tokio::test]
async fn secondary_rate_limit_body_marker_succeeds() {
    let server = MockServer::start().await;
    // Body marker → BackoffThenRetry (500ms sleep in shim).
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string(r#"{"message":"You have exceeded a secondary rate limit."}"#),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"login": "octocat"})),
        )
        .mount(&server)
        .await;

    let cfg = config(&server.uri());
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    conn.healthcheck()
        .await
        .expect("healthcheck should succeed after backoff retry");
}

#[tokio::test]
async fn permission_denied_returns_error_without_retry() {
    let server = MockServer::start().await;
    // Plain 403 → PermissionDenied, no retry. Expect exactly one call.
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string(r#"{"message":"Resource not accessible by integration"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let cfg = config(&server.uri());
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    let err = conn
        .healthcheck()
        .await
        .expect_err("expected error on permission-denied 403");
    let msg = err.to_string();
    assert!(
        msg.contains("permission denied"),
        "expected permission-denied error, got: {msg}",
    );
}
