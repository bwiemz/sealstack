//! End-to-end tests for `send_with_drive_shim` against a wiremock Drive API.
//!
//! Each test verifies a 403 response class produces the right `SealStackError`
//! after the appropriate retry behavior.
//!
//! Note: `start_paused = true` cannot be used here because `HttpClient` builds
//! a reqwest client with a 30s timeout backed by tokio time; pausing the clock
//! causes that timeout to fire before the first HTTP response arrives. The
//! `rate_limit_exhausted` test runs in real time (~7.5s cumulative backoff).

use std::sync::Arc;

use sealstack_common::SealStackError;
use sealstack_connector_google_drive::test_only::send_with_drive_shim;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn body_with_reasons(reasons: &[&str]) -> serde_json::Value {
    let errs: Vec<serde_json::Value> = reasons
        .iter()
        .map(|r| serde_json::json!({"reason": r, "domain": "usageLimits", "message": "x"}))
        .collect();
    serde_json::json!({"error": {"code": 403, "message": "Forbidden", "errors": errs}})
}

fn make_http() -> Arc<HttpClient> {
    Arc::new(HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap())
}

#[tokio::test]
async fn user_rate_limit_retries_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(body_with_reasons(&["userRateLimitExceeded"])),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let resp = send_with_drive_shim(&http, || http.get(&url))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn rate_limit_exhausted_returns_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(body_with_reasons(&["userRateLimitExceeded"])),
        )
        .expect(5) // initial + 4 retries.
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url))
        .await
        .unwrap_err();
    assert!(matches!(err, SealStackError::RateLimited), "got {err:?}");
}

#[tokio::test]
async fn quota_exhausted_returns_rate_limited_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(body_with_reasons(&["quotaExceeded"])),
        )
        .expect(1) // exactly one call — no retries.
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url))
        .await
        .unwrap_err();
    assert!(matches!(err, SealStackError::RateLimited), "got {err:?}");
}

#[tokio::test]
async fn permission_denied_returns_backend_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(403).set_body_json(body_with_reasons(&["forbidden"])))
        .expect(1) // no retries on permission-denied.
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url))
        .await
        .unwrap_err();
    match err {
        SealStackError::Backend(msg) => {
            assert!(msg.contains("permission denied"), "{msg}");
            assert!(msg.contains("forbidden"), "{msg}");
        }
        other => panic!("expected Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn permission_denied_includes_reason() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(body_with_reasons(&[
                "domainPolicy",
                "insufficientPermissions",
            ])),
        )
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url))
        .await
        .unwrap_err();
    match err {
        SealStackError::Backend(msg) => {
            assert!(msg.contains("domainPolicy"), "{msg}");
            assert!(msg.contains("insufficientPermissions"), "{msg}");
        }
        other => panic!("expected Backend, got {other:?}"),
    }
}
