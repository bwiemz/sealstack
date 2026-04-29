//! Integration test for body fetch: export vs alt=media, strict UTF-8, per-file cap.

use std::sync::Arc;

use sealstack_connector_google_drive::test_only::{DriveFileTestStub, fetch_body_for_test};
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_http(_server: &MockServer) -> Arc<HttpClient> {
    Arc::new(
        HttpClient::new(
            Arc::new(StaticToken::new("test-token")),
            RetryPolicy::default(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn fetch_body_exports_google_doc_as_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/doc1/export"))
        .and(query_param("mimeType", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello docs"))
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "doc1".into(),
        mime_type: "application/vnd.google-apps.document".into(),
        size: None,
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, Some("hello docs".to_owned()));
}

#[tokio::test]
async fn fetch_body_direct_downloads_text_plain() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/txt1"))
        .and(query_param("alt", "media"))
        .respond_with(ResponseTemplate::new(200).set_body_string("plain text content"))
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "txt1".into(),
        mime_type: "text/plain".into(),
        size: Some("100".into()),
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, Some("plain text content".to_owned()));
}

#[tokio::test]
async fn fetch_body_skips_unsupported_mime() {
    let server = MockServer::start().await;
    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "img1".into(),
        mime_type: "image/png".into(),
        size: Some("1000".into()),
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, None, "unsupported MIME should yield None");
}

#[tokio::test]
async fn fetch_body_skips_oversized_file() {
    let server = MockServer::start().await;
    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "big1".into(),
        mime_type: "text/plain".into(),
        size: Some("20000000".into()), // 20 MB > 10 MB cap
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, None, "oversized file should yield None");
}

#[tokio::test]
async fn fetch_body_text_with_invalid_utf8_skips() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/badutf"))
        .and(query_param("alt", "media"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xFF_u8, 0xFE, 0xFD]))
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "badutf".into(),
        mime_type: "text/plain".into(),
        size: Some("3".into()),
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        body, None,
        "non-UTF-8 text should be skipped, not lossy-decoded"
    );
}

#[tokio::test]
async fn fetch_body_docs_export_invalid_utf8_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/baddoc/export"))
        .and(query_param("mimeType", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xFF_u8, 0xFE, 0xFD]))
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "baddoc".into(),
        mime_type: "application/vnd.google-apps.document".into(),
        size: None,
    };
    // Docs export contract guarantees UTF-8; a violation is a Google-side
    // bug, not a user-side mistake. Should error rather than silently skip.
    let err = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("non-UTF-8") || err.to_string().contains("docs export"),
        "expected docs-export-non-UTF-8 error, got: {err}"
    );
}
