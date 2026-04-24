//! Integration tests for [`HttpClient`] retry behavior.
//!
//! These tests spin up a local HTTP server via `wiremock`. If CI ever
//! restricts port binding, see `mockito` as the documented fallback.

use std::sync::Arc;
use std::time::Duration;

use sealstack_common::SealStackError;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tight_policy() -> RetryPolicy {
    // Short delays so tests don't drag.
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(200),
    }
}

async fn client(_server: &MockServer) -> HttpClient {
    HttpClient::new(Arc::new(StaticToken::new("t")), tight_policy()).unwrap()
}

#[tokio::test]
async fn happy_path_200() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/ok", server.uri()));
    let resp = c.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn fivehundred_then_ok_retries_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/flaky", server.uri()));
    let resp = c.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn fivehundred_all_attempts_returns_retry_exhausted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/always5xx"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/always5xx", server.uri()));
    let err = c.send(rb).await.unwrap_err();
    match err {
        SealStackError::RetryExhausted { attempts, .. } => {
            assert_eq!(attempts, 4);
        }
        other => panic!("expected RetryExhausted, got {other}"),
    }
}

#[tokio::test]
async fn fourhundred_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bad"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1) // exactly one call — no retries.
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/bad", server.uri()));
    let err = c.send(rb).await.unwrap_err();
    assert!(
        matches!(err, SealStackError::Backend(_)),
        "404 should map to Backend, got {err}"
    );
}

#[tokio::test]
async fn respects_retry_after_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/throttle"))
        .respond_with(
            ResponseTemplate::new(429).append_header("Retry-After", "0"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/throttle"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/throttle", server.uri()));
    let resp = c.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
}
