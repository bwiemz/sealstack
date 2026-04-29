//! End-to-end test for the full list pipeline against a wiremock Drive API.

use std::sync::Arc;

use futures::StreamExt;
use sealstack_connector_google_drive::test_only::list_files_for_test;
use sealstack_connector_sdk::auth::OAuth2Credential;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn list_yields_resources_for_allowlisted_files() {
    let drive_server = MockServer::start().await;
    let token_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ya29.tok",
            "expires_in": 3599,
            "token_type": "Bearer"
        })))
        .mount(&token_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "doc1",
                    "name": "Doc",
                    "mimeType": "application/vnd.google-apps.document",
                    "modifiedTime": "2026-04-27T12:00:00Z",
                    "permissions": [
                        {"type": "anyone", "role": "reader", "allowFileDiscovery": true}
                    ]
                },
                {
                    "id": "bin1",
                    "name": "img.png",
                    "mimeType": "image/png",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                }
            ]
        })))
        .mount(&drive_server)
        .await;

    let cred = Arc::new(
        OAuth2Credential::new(
            "id".to_owned(),
            SecretString::new("s".into()),
            SecretString::new("r".into()),
            format!("{}/token", token_server.uri()),
        )
        .unwrap(),
    );
    let http = Arc::new(HttpClient::new(cred, RetryPolicy::default()).unwrap());

    // Verify paginated walk yields the expected DriveFile ids — only doc1
    // (Doc, allowed); bin1 (image/png) is filtered.
    let mut stream = list_files_for_test(http, &drive_server.uri());
    let mut ids: Vec<String> = Vec::new();
    while let Some(f) = stream.next().await {
        ids.push(f.unwrap().id);
    }
    assert_eq!(ids, vec!["doc1".to_owned()]);
}
