//! Integration tests for [`HttpClient`] retry behavior.
//!
//! These tests spin up a local HTTP server via `wiremock`. If CI ever
//! restricts port binding, see `mockito` as the documented fallback.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use sealstack_common::SealStackError;
use sealstack_connector_sdk::auth::{Credential, StaticToken};
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct CountingCredential {
    invalidations: AtomicU32,
    tokens: Vec<&'static str>,
}

#[async_trait]
impl Credential for CountingCredential {
    async fn authorization_header(&self) -> sealstack_common::SealStackResult<String> {
        let n = self.invalidations.load(Ordering::SeqCst) as usize;
        let t = self.tokens.get(n).unwrap_or(&"exhausted");
        Ok(format!("Bearer {t}"))
    }
    async fn invalidate(&self) {
        self.invalidations.fetch_add(1, Ordering::SeqCst);
    }
}

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

#[tokio::test]
async fn fourhundred_one_triggers_invalidate_and_retries_once() {
    let server = MockServer::start().await;
    // First request: token-1 → 401. Second: token-2 → 200.
    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(wiremock::matchers::header("Authorization", "Bearer token-1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(wiremock::matchers::header("Authorization", "Bearer token-2"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let cred = Arc::new(CountingCredential {
        invalidations: AtomicU32::new(0),
        tokens: vec!["token-1", "token-2"],
    });
    let client = HttpClient::new(cred.clone(), tight_policy()).unwrap();
    let rb = client.get(format!("{}/auth", server.uri()));
    let resp = client.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(cred.invalidations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn second_fourhundred_one_returns_unauthorized() {
    let server = MockServer::start().await;
    // Always 401 regardless of token.
    Mock::given(method("GET"))
        .and(path("/locked"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let cred = Arc::new(CountingCredential {
        invalidations: AtomicU32::new(0),
        tokens: vec!["t1", "t2"],
    });
    let client = HttpClient::new(cred.clone(), tight_policy()).unwrap();
    let rb = client.get(format!("{}/locked", server.uri()));
    let err = client.send(rb).await.unwrap_err();
    assert!(
        matches!(err, SealStackError::Unauthorized(_)),
        "expected Unauthorized, got {err}"
    );
    // Exactly one invalidation; second 401 is final.
    assert_eq!(cred.invalidations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn static_token_401_returns_unauthorized_without_retry_loop() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/locked"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2) // initial + one invalidate-retry; no more.
        .mount(&server)
        .await;

    let cred = Arc::new(StaticToken::new("t"));
    let client = HttpClient::new(cred, tight_policy()).unwrap();
    let rb = client.get(format!("{}/locked", server.uri()));
    let err = client.send(rb).await.unwrap_err();
    assert!(matches!(err, SealStackError::Unauthorized(_)));
}
