//! End-to-end tests for the Slack connector via wiremock.
//!
//! Exercises the full path: `list()` → `list_channels` → `list_messages`
//! → `BodyCursorPaginator` → `HttpClient::send` against a mock Slack API.
//!
//! Each test spins up a `MockServer` on a random local port, builds a
//! `SlackConnector` that points `api_base` at that server, drives `list()`,
//! and asserts the resulting resource count.

use futures::StreamExt;
use sealstack_connector_sdk::Connector;
use sealstack_connector_slack::SlackConnector;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a config that targets the mock server with an optional channel allowlist.
fn config(api_base: &str, channels: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "token": "xoxb-test",
        "api_base": api_base,
        "channels": channels,
        "max_messages_per_channel": 50,
    })
}

#[tokio::test]
async fn list_walks_channels_and_messages() {
    let server = MockServer::start().await;

    // conversations.list: one page, two channels; cursor empty = no more pages.
    Mock::given(method("GET"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "channels": [
                { "id": "C001", "name": "general" },
                { "id": "C002", "name": "random" },
            ],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    // conversations.history for C001: two messages with text.
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1700000000.000100", "user": "U1", "text": "hello" },
                { "ts": "1700000001.000200", "user": "U2", "text": "world" },
            ],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    // The allowlist is ["C001"] so C002's history is never fetched.
    let cfg = config(&server.uri(), &["C001"]);
    let conn = SlackConnector::from_json(&cfg).expect("connector built");
    let mut stream = conn.list().await.expect("list returned stream");

    let mut count = 0usize;
    // ResourceStream yields Resource values directly (not Result<Resource, _>).
    while stream.next().await.is_some() {
        count += 1;
    }

    // list() emits one Resource per message with a non-empty text body.
    // C001 has 2 messages → 2 resources.
    assert_eq!(
        count, 2,
        "expected exactly 2 message resources, got {count}"
    );
}

#[tokio::test]
async fn list_handles_paginated_messages() {
    let server = MockServer::start().await;

    // Single channel — one page of channels.
    Mock::given(method("GET"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "channels": [{ "id": "C001", "name": "general" }],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    // First page of messages — returns cursor "p2".
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C001"))
        .and(wiremock::matchers::query_param_is_missing("cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1700000000.000100", "user": "U1", "text": "msg1" },
                { "ts": "1700000001.000100", "user": "U1", "text": "msg2" },
            ],
            "response_metadata": { "next_cursor": "p2" }
        })))
        .mount(&server)
        .await;

    // Second page — cursor "p2", no further pages.
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C001"))
        .and(query_param("cursor", "p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1700000002.000100", "user": "U1", "text": "msg3" },
            ],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    let cfg = config(&server.uri(), &["C001"]);
    let conn = SlackConnector::from_json(&cfg).expect("connector built");
    let mut stream = conn.list().await.expect("list returned stream");

    let mut count = 0usize;
    while stream.next().await.is_some() {
        count += 1;
    }

    // 2 messages on page 1 + 1 on page 2 = 3 resources total.
    assert_eq!(
        count, 3,
        "expected 3 messages across two pages, got {count}"
    );
}

#[tokio::test]
async fn list_skips_messages_with_empty_text() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "channels": [{ "id": "C001", "name": "general" }],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    // Mix of messages: some empty/missing text, one with real text.
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1700000000.000100", "user": "U1", "text": "" },
                { "ts": "1700000001.000100", "user": "U1" },
                { "ts": "1700000002.000100", "user": "U2", "text": "visible" },
            ],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    let cfg = config(&server.uri(), &["C001"]);
    let conn = SlackConnector::from_json(&cfg).expect("connector built");
    let mut stream = conn.list().await.expect("list returned stream");

    let mut count = 0usize;
    while stream.next().await.is_some() {
        count += 1;
    }

    // Only the third message has non-empty text → 1 resource emitted.
    assert_eq!(count, 1, "expected 1 visible message resource, got {count}");
}

#[tokio::test]
async fn list_channel_allowlist_filters_correctly() {
    let server = MockServer::start().await;

    // Two channels returned by conversations.list.
    Mock::given(method("GET"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "channels": [
                { "id": "C001", "name": "general" },
                { "id": "C002", "name": "secret" },
            ],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    // Only C001 history is mounted — if C002 is fetched, wiremock returns 404.
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .and(query_param("channel", "C001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1700000000.000100", "user": "U1", "text": "only general" },
            ],
            "response_metadata": { "next_cursor": "" }
        })))
        .mount(&server)
        .await;

    // Allowlist contains only C001 — C002 must not be fetched.
    let cfg = config(&server.uri(), &["C001"]);
    let conn = SlackConnector::from_json(&cfg).expect("connector built");
    let mut stream = conn.list().await.expect("list returned stream");

    let mut count = 0usize;
    while stream.next().await.is_some() {
        count += 1;
    }

    assert_eq!(
        count, 1,
        "expected 1 resource from allowed channel, got {count}"
    );
}
