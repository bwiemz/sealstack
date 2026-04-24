use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;

use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{BodyCursorPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize, Debug, PartialEq)]
struct Item {
    id: u32,
}

#[tokio::test]
async fn body_cursor_paginator_walks_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/list"))
        .and(wiremock::matchers::query_param_is_missing("cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "items": [{ "id": 1 }, { "id": 2 }], "next": "p2" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/list"))
        .and(wiremock::matchers::query_param("cursor", "p2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "items": [{ "id": 3 }], "next": null })),
        )
        .mount(&server)
        .await;

    let client =
        Arc::new(HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap());
    let url = format!("{}/list", server.uri());

    let pg = BodyCursorPaginator::<Item, _, _, _>::new(
        move |c: &HttpClient, cursor: Option<&str>| {
            let mut rb = c.get(&url);
            if let Some(cur) = cursor {
                rb = rb.query(&[("cursor", cur)]);
            }
            rb
        },
        |v: &serde_json::Value| {
            let arr = v
                .get("items")
                .and_then(|a| a.as_array())
                .ok_or_else(|| sealstack_common::SealStackError::backend("missing items"))?;
            arr.iter()
                .map(|x| {
                    serde_json::from_value::<Item>(x.clone())
                        .map_err(|e| sealstack_common::SealStackError::backend(format!("{e}")))
                })
                .collect()
        },
        |v: &serde_json::Value| v.get("next").and_then(|c| c.as_str()).map(str::to_owned),
    );
    let items: Vec<_> = paginate(pg, client).collect().await;
    let ok: Vec<Item> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(ok, vec![Item { id: 1 }, Item { id: 2 }, Item { id: 3 }]);
}
