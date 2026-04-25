//! End-to-end tests for the GitHub connector via wiremock.
//!
//! Exercises the full path: `list()` → `list_repos` → `LinkHeaderPaginator`
//! → `HttpClient::send` against a mock GitHub API.
//!
//! Each test spins up a `MockServer` on a random local port, builds a
//! `GithubConnector` that points `api_base` at that server, and drives
//! `list()` to completion.

use futures::StreamExt;
use sealstack_connector_github::GithubConnector;
use sealstack_connector_sdk::Connector;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(api_base: &str) -> serde_json::Value {
    serde_json::json!({
        "token": "ghp_test_token",
        "api_base": api_base,
        "include_issues": false,
    })
}

/// A minimal repo JSON object with the required `updated_at` RFC3339 field.
fn repo_json(name: &str, owner: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "owner": { "login": owner },
        "updated_at": "2024-01-15T12:00:00Z",
    })
}

#[tokio::test]
async fn list_walks_repos_via_link_header_pagination() {
    let server = MockServer::start().await;
    let base = server.uri();
    // The full URL that will appear in the Link: rel="next" header.
    let page2_url = format!("{base}/user/repos?per_page=100&affiliation=owner&page=2");

    // First page — two repos + Link header pointing to page 2.
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([
                    repo_json("repo-a", "octocat"),
                    repo_json("repo-b", "octocat"),
                ]))
                .append_header("Link", format!("<{page2_url}>; rel=\"next\"")),
        )
        .mount(&server)
        .await;

    // Second page — one more repo, no Link header.
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([repo_json("repo-c", "octocat"),])),
        )
        .mount(&server)
        .await;

    // README requests for all three repos — return 404 to avoid fixture work.
    // The connector treats 404 as Ok(None) → no resource emitted per repo.
    Mock::given(method("GET"))
        .and(wiremock::matchers::path_regex(
            r"/repos/octocat/repo-[abc]/readme",
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found",
            "documentation_url": "https://docs.github.com"
        })))
        .mount(&server)
        .await;

    let cfg = config(&base);
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    // ResourceStream yields Resource values directly (not Result<Resource, _>).
    let resources: Vec<_> = conn
        .list()
        .await
        .expect("list returned stream")
        .collect()
        .await;

    // All three repos have 404 readmes → list() emits 0 readme resources.
    // The key regression test: 404 must not crash the stream.
    assert_eq!(
        resources.len(),
        0,
        "expected 0 resources (all readmes 404), got {}",
        resources.len()
    );
}

#[tokio::test]
async fn list_emits_readme_resources_when_present() {
    let server = MockServer::start().await;
    let base = server.uri();

    // Single-page repo list — one repo.
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([repo_json("docs", "octocat")])),
        )
        .mount(&server)
        .await;

    // README for docs — valid base64-encoded content "# Docs".
    // "IyBEb2Nz" is base64("# Docs").
    Mock::given(method("GET"))
        .and(path("/repos/octocat/docs/readme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": "IyBEb2Nz\n",
            "encoding": "base64",
        })))
        .mount(&server)
        .await;

    let cfg = config(&base);
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    let resources: Vec<_> = conn
        .list()
        .await
        .expect("list returned stream")
        .collect()
        .await;

    // One repo with a valid README → 1 readme resource.
    assert_eq!(
        resources.len(),
        1,
        "expected 1 readme resource, got {}",
        resources.len()
    );
    assert_eq!(resources[0].kind, "readme");
    assert!(
        resources[0].body.contains("Docs"),
        "readme body should contain decoded content, got: {}",
        resources[0].body
    );
}

#[tokio::test]
async fn list_pagination_three_repos_across_two_pages() {
    let server = MockServer::start().await;
    let base = server.uri();
    let page2_url = format!("{base}/user/repos?per_page=100&affiliation=owner&page=2");

    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([
                    repo_json("alpha", "org"),
                    repo_json("beta", "org"),
                ]))
                .append_header("Link", format!("<{page2_url}>; rel=\"next\"")),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([repo_json("gamma", "org"),])),
        )
        .mount(&server)
        .await;

    // Serve a README for each repo so we can verify all three were visited.
    // "IyBSZWFkbWU=" is base64("# Readme").
    for name in &["alpha", "beta", "gamma"] {
        Mock::given(method("GET"))
            .and(path(format!("/repos/org/{name}/readme")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "IyBSZWFkbWU=",
                "encoding": "base64",
            })))
            .mount(&server)
            .await;
    }

    let cfg = config(&base);
    let conn = GithubConnector::from_json(&cfg).expect("connector built");
    let resources: Vec<_> = conn
        .list()
        .await
        .expect("list returned stream")
        .collect()
        .await;

    // 3 repos × 1 readme each = 3 resources.
    assert_eq!(
        resources.len(),
        3,
        "expected 3 readme resources (one per repo), got {}",
        resources.len()
    );

    // Verify resource ids carry the repo names.
    let ids: Vec<String> = resources.iter().map(|r| r.id.to_string()).collect();
    assert!(
        ids.iter().any(|id| id.contains("alpha")),
        "missing alpha in ids: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id.contains("beta")),
        "missing beta in ids: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id.contains("gamma")),
        "missing gamma in ids: {ids:?}"
    );
}
