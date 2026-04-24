use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;

use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{OffsetPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize, Debug, PartialEq)]
struct Row {
    id: u32,
}

#[tokio::test]
async fn offset_paginator_walks_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rows"))
        .and(wiremock::matchers::query_param("startAt", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "startAt": 0, "maxResults": 2, "total": 3,
            "values": [{ "id": 1 }, { "id": 2 }],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rows"))
        .and(wiremock::matchers::query_param("startAt", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "startAt": 2, "maxResults": 2, "total": 3,
            "values": [{ "id": 3 }],
        })))
        .mount(&server)
        .await;

    let client =
        Arc::new(HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap());
    let url = format!("{}/rows", server.uri());

    let pg = OffsetPaginator::<Row, _>::new(
        2,
        move |c: &HttpClient, start: u64, limit: u64| {
            c.get(&url).query(&[
                ("startAt", start.to_string()),
                ("maxResults", limit.to_string()),
            ])
        },
        "values",
    );
    let items: Vec<_> = paginate(pg, client).collect().await;
    let ok: Vec<Row> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(ok, vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
}
