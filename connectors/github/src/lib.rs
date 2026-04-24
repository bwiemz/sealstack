//! GitHub connector.
//!
//! Pulls repository READMEs and issues via the GitHub REST API. Authentication
//! is a personal access token (PAT) passed in construction; v0.2 will swap in
//! GitHub App installation tokens. Supports filtering by a user/org login and
//! by repository name.
//!
//! # Resources emitted
//!
//! * One `readme` resource per repository, with the rendered README text as
//!   the body.
//! * One `issue` resource per open+closed issue, with the issue body as the
//!   body and the title surfaced separately.
//!
//! Anything beyond that (PRs, discussions, comments) is deferred to v0.2.
//!
//! # Pagination
//!
//! GitHub's REST API paginates via `Link: <...>; rel="next"` headers. The
//! connector walks until the header is absent. Per-page size is capped by
//! GitHub at 100.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod retry_shim;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::{HttpClient, HttpResponse};
use sealstack_connector_sdk::paginate::{LinkHeaderPaginator, next_link, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const GITHUB_API: &str = "https://api.github.com";

/// Configuration for the GitHub connector (non-secret fields only).
#[derive(Clone, Debug)]
pub struct GithubConfig {
    /// The user or org login to pull from (e.g. `"sealstack"`). When
    /// `None`, pulls every repo the token can see.
    pub owner: Option<String>,
    /// Optional comma-separated allow-list of repo names. Empty = all.
    pub repos: Vec<String>,
    /// Whether to pull issue bodies in addition to READMEs. Defaults true.
    pub include_issues: bool,
}

impl GithubConfig {
    /// Parse non-secret fields from the binding config JSON shape:
    ///
    /// ```json
    /// { "owner": "acme", "repos": ["docs", "core"], "include_issues": true }
    /// ```
    #[must_use]
    pub fn from_json(v: &serde_json::Value) -> Self {
        let owner = v.get("owner").and_then(|x| x.as_str()).map(str::to_owned);
        let repos: Vec<String> = v
            .get("repos")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let include_issues = v
            .get("include_issues")
            .and_then(|x| x.as_bool())
            .unwrap_or(true);
        Self {
            owner,
            repos,
            include_issues,
        }
    }
}

/// At most: initial attempt + one shim-guided retry.
async fn send_with_gh_shim<F>(http: &HttpClient, make_request: F) -> SealStackResult<HttpResponse>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    for attempt in 0..2u8 {
        match http.send(make_request()).await {
            Ok(resp) => return Ok(resp),
            Err(SealStackError::HttpStatus {
                status: 403,
                headers,
                body,
            }) => match retry_shim::classify_github_403(&headers, &body) {
                retry_shim::Github403Action::WaitThenRetry(d) if attempt == 0 => {
                    tracing::warn!(?d, "github: 403 rate-limit, waiting before retry");
                    tokio::time::sleep(d).await;
                }
                retry_shim::Github403Action::BackoffThenRetry if attempt == 0 => {
                    tracing::warn!("github: 403 secondary rate-limit, backing off before retry");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                retry_shim::Github403Action::PermissionDenied => {
                    return Err(SealStackError::Backend(
                        "github 403: permission denied".into(),
                    ));
                }
                _ => {
                    return Err(SealStackError::Backend(
                        "github 403: rate-limit retry exhausted".into(),
                    ));
                }
            },
            Err(e) => return Err(e),
        }
    }
    Err(SealStackError::Backend(
        "github: 403 retry loop terminated unexpectedly".into(),
    ))
}

/// The GitHub connector.
#[derive(Debug)]
pub struct GithubConnector {
    http: Arc<HttpClient>,
    config: GithubConfig,
}

impl GithubConnector {
    /// Build a connector from a JSON config value.
    ///
    /// Token precedence:
    /// - Config `token` field present → use it (warns if `GITHUB_TOKEN` is
    ///   also set).
    /// - Config absent + `GITHUB_TOKEN` env var present and non-empty → use
    ///   env var.
    /// - Both absent or empty → returns `SealStackError::Config`.
    ///
    /// Token is not validated against the API until the first call.
    pub fn from_json(v: &serde_json::Value) -> SealStackResult<Self> {
        let token = match (
            v.get("token").and_then(|x| x.as_str()),
            std::env::var("GITHUB_TOKEN").ok(),
        ) {
            (Some(t), env_present) => {
                if env_present.is_some() {
                    tracing::warn!("github: config `token` set; GITHUB_TOKEN env ignored");
                }
                t.to_owned()
            }
            (None, Some(env)) if !env.is_empty() => env,
            _ => {
                return Err(SealStackError::Config(
                    "github connector requires `token` in config or GITHUB_TOKEN env".into(),
                ));
            }
        };

        let credential = Arc::new(StaticToken::new(token));
        let http = Arc::new(
            HttpClient::new(credential, RetryPolicy::default())?
                .with_user_agent_suffix(format!("github-connector/{}", env!("CARGO_PKG_VERSION"))),
        );
        let config = GithubConfig::from_json(v);
        Ok(Self { http, config })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
    ) -> SealStackResult<(T, Option<String>)> {
        let make = || {
            self.http
                .get(url)
                .header("accept", "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28")
        };

        let resp = send_with_gh_shim(&self.http, make).await?;

        // Capture the Link header before consuming the response.
        let next = resp.header("link").and_then(next_link);

        let value = resp.json::<T>().await?;
        Ok((value, next))
    }

    async fn list_repos(&self) -> SealStackResult<Vec<Repo>> {
        let initial = match &self.config.owner {
            Some(o) => format!("{GITHUB_API}/users/{o}/repos?per_page=100&type=owner"),
            None => format!("{GITHUB_API}/user/repos?per_page=100&affiliation=owner"),
        };
        let pg =
            LinkHeaderPaginator::<Repo, _>::new(move |c: &HttpClient, cursor: Option<&str>| {
                let url = cursor.unwrap_or(initial.as_str());
                c.get(url)
                    .header("accept", "application/vnd.github+json")
                    .header("x-github-api-version", "2022-11-28")
            });
        let mut stream = paginate(pg, self.http.clone());
        let allow = &self.config.repos;
        let mut out: Vec<Repo> = Vec::new();
        while let Some(item) = stream.next().await {
            let r = item?;
            if allow.is_empty() || allow.iter().any(|w| w == &r.name) {
                out.push(r);
            }
        }
        Ok(out)
    }

    async fn fetch_readme(&self, owner: &str, repo: &str) -> SealStackResult<Option<String>> {
        let url = format!("{GITHUB_API}/repos/{owner}/{repo}/readme");
        match self.get_json::<ReadmeBody>(&url).await {
            Ok((body, _)) => Ok(Some(decode_base64_content(&body.content))),
            Err(SealStackError::Backend(m)) if m.contains("404") => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn list_issues(&self, owner: &str, repo: &str) -> SealStackResult<Vec<Issue>> {
        let initial = format!("{GITHUB_API}/repos/{owner}/{repo}/issues?state=all&per_page=100");
        let pg =
            LinkHeaderPaginator::<Issue, _>::new(move |c: &HttpClient, cursor: Option<&str>| {
                let url = cursor.unwrap_or(initial.as_str());
                c.get(url)
                    .header("accept", "application/vnd.github+json")
                    .header("x-github-api-version", "2022-11-28")
            });
        let mut stream = paginate(pg, self.http.clone());
        let mut out: Vec<Issue> = Vec::new();
        while let Some(item) = stream.next().await {
            let issue = item?;
            // The issues endpoint returns PRs too — filter them out.
            if issue.pull_request.is_none() {
                out.push(issue);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Connector for GithubConnector {
    fn name(&self) -> &str {
        "github"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let repos = self.list_repos().await?;
        let mut out: Vec<Resource> = Vec::new();

        for repo in repos {
            let owner = repo.owner.login.clone();
            let updated = parse_time(&repo.updated_at).unwrap_or_else(OffsetDateTime::now_utc);
            let perms = vec![PermissionPredicate {
                principal: format!("github:{}", owner),
                action: "read".into(),
            }];

            if let Some(readme) = self.fetch_readme(&owner, &repo.name).await? {
                out.push(Resource {
                    id: ResourceId::new(format!("github://{owner}/{}/readme", repo.name)),
                    kind: "readme".into(),
                    title: Some(format!("{}/{}", owner, repo.name)),
                    body: readme,
                    metadata: serde_json::Map::from_iter([(
                        "repo".into(),
                        serde_json::Value::String(format!("{owner}/{}", repo.name)),
                    )]),
                    permissions: perms.clone(),
                    source_updated_at: updated,
                });
            }

            if self.config.include_issues {
                for issue in self.list_issues(&owner, &repo.name).await? {
                    let iu = parse_time(&issue.updated_at).unwrap_or_else(OffsetDateTime::now_utc);
                    out.push(Resource {
                        id: ResourceId::new(format!(
                            "github://{owner}/{}/issues/{}",
                            repo.name, issue.number
                        )),
                        kind: "issue".into(),
                        title: Some(issue.title.clone()),
                        body: issue.body.unwrap_or_default(),
                        metadata: serde_json::Map::from_iter([
                            (
                                "repo".into(),
                                serde_json::Value::String(format!("{owner}/{}", repo.name)),
                            ),
                            ("number".into(), serde_json::Value::from(issue.number)),
                            ("state".into(), serde_json::Value::String(issue.state)),
                        ]),
                        permissions: perms.clone(),
                        source_updated_at: iu,
                    });
                }
            }
        }

        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        // v0.1 does not resolve single-resource fetches by id; the full list
        // path is sufficient for initial sync. On-demand fetch lands when
        // webhook-driven push is wired.
        Err(SealStackError::NotFound(format!(
            "github fetch not yet implemented for `{id}`"
        )))
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let url = format!("{GITHUB_API}/user");
        let _: (serde_json::Value, _) = self.get_json(&url).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Repo {
    name: String,
    owner: Owner,
    updated_at: String,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
}

#[derive(Deserialize)]
struct ReadmeBody {
    /// Base64-encoded content with `\n` line breaks every 60 chars.
    content: String,
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    updated_at: String,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_time(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &Rfc3339).ok()
}

/// Decode GitHub's base64 content string (with embedded `\n` chars).
fn decode_base64_content(raw: &str) -> String {
    use base64_lite::decode;
    let joined: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    match decode(&joined) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    }
}

// Minimal inline base64 decoder — kept in-crate to avoid pulling a full
// `base64` dependency just for README content. GitHub emits standard base64.
mod base64_lite {
    pub(crate) fn decode(s: &str) -> Result<Vec<u8>, &'static str> {
        static ALPHABET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        if bytes.len() % 4 != 0 {
            return Err("base64: length not multiple of 4");
        }
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        let mut val: u32 = 0;
        let mut bits = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'=' {
                break;
            }
            let idx = ALPHABET
                .iter()
                .position(|&a| a == b)
                .ok_or("base64: invalid char")?;
            val = (val << 6) | idx as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((val >> bits) & 0xff) as u8);
            }
            // Silence the unused-index warning under -Wunused.
            let _ = i;
        }
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::decode;

        #[test]
        fn decodes_hello() {
            assert_eq!(decode("aGVsbG8=").unwrap(), b"hello".to_vec());
        }

        #[test]
        fn decodes_empty() {
            assert_eq!(decode("").unwrap(), Vec::<u8>::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_json_accepts_minimal_shape() {
        let v = serde_json::json!({ "token": "t" });
        let c = GithubConnector::from_json(&v).unwrap();
        assert!(c.config.owner.is_none());
        assert!(c.config.repos.is_empty());
        assert!(c.config.include_issues);
    }

    #[test]
    fn config_from_json_respects_owner_and_repos() {
        let v = serde_json::json!({
            "token": "t",
            "owner": "acme",
            "repos": ["docs"],
            "include_issues": false,
        });
        let c = GithubConnector::from_json(&v).unwrap();
        assert_eq!(c.config.owner.as_deref(), Some("acme"));
        assert_eq!(c.config.repos, vec!["docs".to_string()]);
        assert!(!c.config.include_issues);
    }

    #[test]
    fn from_json_missing_token_errors() {
        // This test only works when GITHUB_TOKEN is unset. Skip if the var
        // is present in the environment (CI may set it).
        if std::env::var("GITHUB_TOKEN").is_ok() {
            return;
        }
        let v = serde_json::json!({});
        match GithubConnector::from_json(&v) {
            Err(SealStackError::Config(_)) => {}
            other => panic!("expected Config error, got {other:?}"),
        }
    }
}
