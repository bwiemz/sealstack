use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;

use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{LinkHeaderPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize, Debug, PartialEq)]
struct Issue {
    id: u32,
}

#[tokio::test]
async fn link_header_paginator_walks_pages() {
    let server = MockServer::start().await;
    let next_url = format!("{}/issues?page=2", server.uri());

    Mock::given(method("GET"))
        .and(path("/issues"))
        .and(wiremock::matchers::query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([{ "id": 1 }, { "id": 2 }]))
                .append_header("Link", format!("<{next_url}>; rel=\"next\"")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/issues"))
        .and(wiremock::matchers::query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "id": 3 }])))
        .mount(&server)
        .await;

    let client =
        Arc::new(HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap());
    let initial = format!("{}/issues", server.uri());

    let pg = LinkHeaderPaginator::<Issue, _>::new(move |c: &HttpClient, cursor: Option<&str>| {
        match cursor {
            None => c.get(&initial),
            Some(url) => c.get(url),
        }
    });
    let items: Vec<_> = paginate(pg, client).collect().await;
    let ok: Vec<Issue> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(ok, vec![Issue { id: 1 }, Issue { id: 2 }, Issue { id: 3 }]);
}
