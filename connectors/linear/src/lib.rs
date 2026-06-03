//! Linear GraphQL connector.
//!
//! Lists workspace issues via the Linear GraphQL `issues` connection. Each
//! issue is emitted as one [`Resource`] with the markdown description as
//! the body. Pagination uses Relay-style `pageInfo { hasNextPage endCursor }`.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "api_key":   "lin_api_xxx",
//!   "page_size": 50,
//!   "max_issues": 1000
//! }
//! ```
//!
//! `api_key` is a personal or workspace API key from Linear's settings →
//! API. OAuth is out of scope for v0.4.
//!
//! # Limitations (v0.4)
//!
//! - Issues only — projects, cycles, documents, and comments are not pulled.
//!   Issue descriptions are markdown; comment threads are summarized only
//!   by issue title for now.
//! - Permissions are emitted as `linear:team:<key>` group predicates. Linear
//!   itself has finer-grained access controls (private team views, guest
//!   roles); we surface the team-key gating as a coarse approximation
//!   suitable for the receipt trail.

use async_trait::async_trait;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Principal, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use time::OffsetDateTime;

const DEFAULT_API_URL: &str = "https://api.linear.app/graphql";
const DEFAULT_PAGE_SIZE: u32 = 50;
const DEFAULT_MAX_ISSUES: u32 = 1000;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const USER_AGENT: &str = concat!("sealstack-linear/", env!("CARGO_PKG_VERSION"));

const ISSUES_QUERY: &str = r"
query Issues($first: Int!, $after: String) {
  issues(first: $first, after: $after) {
    nodes {
      id
      identifier
      title
      description
      url
      updatedAt
      archivedAt
      team { key name }
      state { name type }
    }
    pageInfo { hasNextPage endCursor }
  }
}
";

/// Connector configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Linear API key (`lin_api_...`).
    pub api_key: String,
    /// Optional GraphQL endpoint override — defaults to the public Linear API.
    #[serde(default)]
    pub api_url: Option<String>,
    /// Per-page count for the `issues` connection.
    #[serde(default)]
    pub page_size: Option<u32>,
    /// Cap on total issues enumerated per sync.
    #[serde(default)]
    pub max_issues: Option<u32>,
    /// Per-request timeout, seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// Linear connector.
#[derive(Clone, Debug)]
pub struct LinearConnector {
    client: reqwest::Client,
    api_url: String,
    page_size: u32,
    max_issues: u32,
}

impl LinearConnector {
    /// Build from a JSON config payload.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] on parse / unknown-field error or
    /// a missing/malformed token. Returns [`SealStackError::Backend`] if the
    /// reqwest client cannot be built.
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("linear connector config: {e}")))?;
        Self::new(config)
    }

    /// Build from a typed [`Config`].
    ///
    /// # Errors
    ///
    /// See [`Self::from_json`].
    pub fn new(config: Config) -> SealStackResult<Self> {
        if config.api_key.is_empty() {
            return Err(SealStackError::Config(
                "linear connector requires a non-empty `api_key`".into(),
            ));
        }
        let api_url = config
            .api_url
            .clone()
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        let page_size = config.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        let max_issues = config.max_issues.unwrap_or(DEFAULT_MAX_ISSUES);
        let timeout =
            Duration::from_secs(config.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));

        let mut headers = reqwest::header::HeaderMap::new();
        let auth_header = reqwest::header::HeaderValue::from_str(&config.api_key).map_err(|e| {
            SealStackError::Config(format!("api_key has invalid header characters: {e}"))
        })?;
        headers.insert(reqwest::header::AUTHORIZATION, auth_header);
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .default_headers(headers)
            .build()
            .map_err(|e| SealStackError::Backend(format!("reqwest client: {e}")))?;

        drop(config);
        Ok(Self {
            client,
            api_url,
            page_size,
            max_issues,
        })
    }

    async fn list_issues(&self) -> SealStackResult<Vec<LinearIssue>> {
        let mut out: Vec<LinearIssue> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            if out.len() >= self.max_issues as usize {
                break;
            }
            let body = json!({
                "query": ISSUES_QUERY,
                "variables": {
                    "first": self.page_size,
                    "after": cursor,
                },
            });

            let resp = self
                .client
                .post(&self.api_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| SealStackError::Backend(format!("linear graphql: {e}")))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SealStackError::Unauthorized(
                    "linear: api_key rejected".into(),
                ));
            }
            if !status.is_success() {
                return Err(SealStackError::Backend(format!(
                    "linear graphql: HTTP {status}",
                )));
            }
            let envelope: GqlEnvelope = resp
                .json()
                .await
                .map_err(|e| SealStackError::Backend(format!("linear graphql body: {e}")))?;
            if let Some(errs) = envelope.errors
                && !errs.is_empty()
            {
                let first = errs
                    .first()
                    .and_then(|e| e.get("message").and_then(Value::as_str))
                    .unwrap_or("unknown linear graphql error");
                return Err(SealStackError::Backend(format!("linear graphql: {first}",)));
            }
            let Some(data) = envelope.data else {
                return Err(SealStackError::Backend("linear graphql: empty data".into()));
            };
            for node in data.issues.nodes {
                out.push(node);
                if out.len() >= self.max_issues as usize {
                    break;
                }
            }
            if !data.issues.page_info.has_next_page {
                break;
            }
            cursor = data.issues.page_info.end_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Connector for LinearConnector {
    fn name(&self) -> &str {
        "linear"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let issues = self.list_issues().await?;
        let mut out: Vec<Resource> = Vec::with_capacity(issues.len());
        for issue in issues {
            out.push(issue_to_resource(&issue));
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        let identifier = id.as_str().strip_prefix("linear://issue/").ok_or_else(|| {
            SealStackError::NotFound(format!("id `{id}` is not a linear issue reference"))
        })?;

        let body = json!({
            "query": r"
                query Issue($id: String!) {
                  issue(id: $id) {
                    id identifier title description url updatedAt archivedAt
                    team { key name } state { name type }
                  }
                }
            ",
            "variables": { "id": identifier },
        });
        let resp = self
            .client
            .post(&self.api_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("linear issue: {e}")))?;
        if !resp.status().is_success() {
            return Err(SealStackError::Backend(format!(
                "linear issue: HTTP {}",
                resp.status(),
            )));
        }
        let raw: Value = resp
            .json()
            .await
            .map_err(|e| SealStackError::Backend(format!("linear issue body: {e}")))?;
        let Some(node) = raw.get("data").and_then(|d| d.get("issue")) else {
            return Err(SealStackError::NotFound(format!(
                "linear issue {identifier}"
            )));
        };
        let issue: LinearIssue = serde_json::from_value(node.clone())
            .map_err(|e| SealStackError::Backend(format!("linear issue parse: {e}")))?;
        Ok(issue_to_resource(&issue))
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let body = json!({ "query": "query { viewer { id } }" });
        let resp = self
            .client
            .post(&self.api_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("linear healthcheck: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SealStackError::Unauthorized(
                "linear: api_key rejected".into(),
            ));
        }
        if !resp.status().is_success() {
            return Err(SealStackError::Backend(format!(
                "linear healthcheck: HTTP {}",
                resp.status(),
            )));
        }
        Ok(())
    }
}

fn issue_to_resource(issue: &LinearIssue) -> Resource {
    let updated = issue
        .updated_at
        .as_deref()
        .and_then(parse_iso8601)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let team_key = issue
        .team
        .as_ref()
        .map(|t| t.key.clone())
        .unwrap_or_else(|| "unknown".into());
    let mut metadata = serde_json::Map::new();
    metadata.insert("linear_id".into(), Value::String(issue.id.clone()));
    metadata.insert("identifier".into(), Value::String(issue.identifier.clone()));
    if let Some(url) = &issue.url {
        metadata.insert("url".into(), Value::String(url.clone()));
    }
    if let Some(state) = &issue.state {
        metadata.insert("state".into(), Value::String(state.name.clone()));
        metadata.insert("state_type".into(), Value::String(state.r#type.clone()));
    }
    metadata.insert("archived".into(), Value::Bool(issue.archived_at.is_some()));

    Resource {
        id: ResourceId::new(format!("linear://issue/{}", issue.id)),
        kind: "issue".into(),
        title: Some(issue.title.clone()),
        body: issue.description.clone().unwrap_or_default(),
        metadata,
        permissions: vec![PermissionPredicate {
            principal: Principal::Group(format!("linear:team:{team_key}")),
            action: "read".into(),
        }],
        source_updated_at: updated,
    }
}

#[derive(Debug, Deserialize)]
struct GqlEnvelope {
    #[serde(default)]
    data: Option<GqlData>,
    #[serde(default)]
    errors: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct GqlData {
    issues: GqlIssuesConnection,
}

#[derive(Debug, Deserialize)]
struct GqlIssuesConnection {
    #[serde(default)]
    nodes: Vec<LinearIssue>,
    #[serde(rename = "pageInfo")]
    page_info: GqlPageInfo,
}

#[derive(Debug, Deserialize)]
struct GqlPageInfo {
    #[serde(rename = "hasNextPage", default)]
    has_next_page: bool,
    #[serde(rename = "endCursor", default)]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct LinearIssue {
    id: String,
    identifier: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "updatedAt", default)]
    updated_at: Option<String>,
    #[serde(rename = "archivedAt", default)]
    archived_at: Option<String>,
    #[serde(default)]
    team: Option<LinearTeam>,
    #[serde(default)]
    state: Option<LinearState>,
}

#[derive(Debug, Deserialize, Clone)]
struct LinearTeam {
    key: String,
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
}

#[derive(Debug, Deserialize, Clone)]
struct LinearState {
    name: String,
    r#type: String,
}

fn parse_iso8601(s: &str) -> Option<OffsetDateTime> {
    let fmt = time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(s, &fmt).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_empty_key() {
        let json = serde_json::json!({ "api_key": "" });
        let err = LinearConnector::from_json(&json).expect_err("empty rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unknown_field() {
        let json = serde_json::json!({ "api_key": "lin_api_x", "spelunk": true });
        let err = LinearConnector::from_json(&json).expect_err("unknown rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_accepts_minimal() {
        let json = serde_json::json!({ "api_key": "lin_api_x" });
        let c = LinearConnector::from_json(&json).expect("builds");
        assert_eq!(c.api_url, DEFAULT_API_URL);
        assert_eq!(c.page_size, DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn issue_to_resource_preserves_team_and_state() {
        let issue = LinearIssue {
            id: "abc-1".into(),
            identifier: "ENG-7".into(),
            title: "Fix retry".into(),
            description: Some("Body".into()),
            url: Some("https://linear.app/x/issue/ENG-7".into()),
            updated_at: Some("2024-01-02T03:04:05.000Z".into()),
            archived_at: None,
            team: Some(LinearTeam {
                key: "ENG".into(),
                name: "Engineering".into(),
            }),
            state: Some(LinearState {
                name: "In Progress".into(),
                r#type: "started".into(),
            }),
        };
        let r = issue_to_resource(&issue);
        assert_eq!(r.id.as_str(), "linear://issue/abc-1");
        assert_eq!(r.title.as_deref(), Some("Fix retry"));
        assert_eq!(r.body, "Body");
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "linear:team:ENG",
        ));
        assert_eq!(
            r.metadata.get("identifier").and_then(Value::as_str),
            Some("ENG-7")
        );
        assert_eq!(
            r.metadata.get("state").and_then(Value::as_str),
            Some("In Progress")
        );
    }

    #[test]
    fn issue_to_resource_handles_archived_and_missing_team() {
        let issue = LinearIssue {
            id: "x".into(),
            identifier: "X-1".into(),
            title: "t".into(),
            description: None,
            url: None,
            updated_at: None,
            archived_at: Some("2024-01-01T00:00:00Z".into()),
            team: None,
            state: None,
        };
        let r = issue_to_resource(&issue);
        assert_eq!(r.body, "");
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "linear:team:unknown",
        ));
        assert_eq!(
            r.metadata.get("archived").and_then(Value::as_bool),
            Some(true)
        );
    }
}
