//! Integration test for `files.list` pagination + MIME allowlist + driveId skip.

use std::sync::Arc;

use futures::StreamExt;
use sealstack_connector_google_drive::test_only::list_files_for_test;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn list_files_walks_pages_and_filters() {
    let server = MockServer::start().await;
    // First page: 3 files. One Doc (allowed), one binary (skipped via MIME),
    // one Shared Drive item (driveId set, skipped via corpora=user constraint).
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::query_param_is_missing("pageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "doc1",
                    "name": "Design Doc",
                    "mimeType": "application/vnd.google-apps.document",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                },
                {
                    "id": "bin1",
                    "name": "image.png",
                    "mimeType": "image/png",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                },
                {
                    "id": "shared1",
                    "name": "shared.md",
                    "mimeType": "text/markdown",
                    "modifiedTime": "2026-04-27T12:00:00Z",
                    "driveId": "0AABCDEF12345"
                }
            ],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    // Second page: 1 markdown file. No nextPageToken.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "md1",
                    "name": "README.md",
                    "mimeType": "text/markdown",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                }
            ]
        })))
        .mount(&server)
        .await;

    let http = Arc::new(
        HttpClient::new(
            Arc::new(StaticToken::new("test-token")),
            RetryPolicy::default(),
        )
        .unwrap(),
    );
    let api_base = server.uri();
    let mut stream = list_files_for_test(http.clone(), &api_base);
    let mut ids: Vec<String> = Vec::new();
    while let Some(file) = stream.next().await {
        ids.push(file.unwrap().id);
    }
    // Expected: doc1 (Doc, allowed), md1 (markdown, allowed). bin1 filtered
    // (image/png not in allowlist), shared1 filtered (driveId set → v1 corpora=user).
    assert_eq!(ids, vec!["doc1".to_owned(), "md1".to_owned()]);
}
