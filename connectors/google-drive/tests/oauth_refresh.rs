//! LOAD-BEARING ACCEPTANCE CRITERION (spec §10).
//!
//! If this test passes, the slice has done its load-bearing job. If it
//! fails, no other test result matters.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sealstack_common::SealStackError;
use sealstack_connector_sdk::auth::OAuth2Credential;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn oauth_refresh_on_401_succeeds_on_retry() {
    let drive_server = MockServer::start().await;
    let token_server = MockServer::start().await;

    // Token endpoint: first call returns ya29.first; subsequent calls return ya29.second.
    let issued_second = Arc::new(AtomicBool::new(false));
    let flag = issued_second.clone();
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(move |_: &wiremock::Request| {
            let n = if flag.load(Ordering::SeqCst) {
                "ya29.second"
            } else {
                "ya29.first"
            };
            flag.store(true, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": n,
                "expires_in": 3599,
                "token_type": "Bearer"
            }))
        })
        .mount(&token_server)
        .await;

    // Drive: first request with `Bearer ya29.first` → 401; second with `Bearer ya29.second` → 200.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer ya29.first",
        ))
        .respond_with(
            ResponseTemplate::new(401)
                .append_header("WWW-Authenticate", r#"Bearer error="invalid_token""#),
        )
        .mount(&drive_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer ya29.second",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": []
        })))
        .mount(&drive_server)
        .await;

    // Build OAuth2Credential pointing at the wiremock token endpoint.
    // We bypass DriveConnector::from_json because OAuth2Credential::google()
    // hardcodes the real Google token endpoint; for this test we want to
    // direct token traffic to wiremock.
    let cred = Arc::new(
        OAuth2Credential::new(
            "test-client".to_owned(),
            SecretString::new("secret".into()),
            SecretString::new("refresh-value".into()),
            format!("{}/token", token_server.uri()),
        )
        .unwrap(),
    );
    let http = Arc::new(
        HttpClient::new(cred, RetryPolicy::default())
            .unwrap()
            .with_user_agent_suffix("oauth-refresh-test/0.1.0"),
    );

    // Drive endpoint with a 401 → invalidate-once → refresh → 200.
    let url = format!("{}/drive/v3/files", drive_server.uri());
    let resp = http.send(http.get(&url)).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn oauth_refresh_invalid_grant_surfaces_unauthorized() {
    let token_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked."
        })))
        .mount(&token_server)
        .await;

    let cred = Arc::new(
        OAuth2Credential::new(
            "test-client".to_owned(),
            SecretString::new("secret".into()),
            SecretString::new("revoked-refresh".into()),
            format!("{}/token", token_server.uri()),
        )
        .unwrap(),
    );
    let http = HttpClient::new(cred, RetryPolicy::default()).unwrap();

    let drive_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anywhere"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&drive_server)
        .await;

    let url = format!("{}/anywhere", drive_server.uri());
    let err = http.send(http.get(&url)).await.unwrap_err();
    match err {
        SealStackError::Unauthorized(msg) => {
            assert!(msg.contains("invalid_grant"), "{msg}");
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}
