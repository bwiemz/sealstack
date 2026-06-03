//! Jira Cloud REST connector.
//!
//! Pulls issues via `GET /rest/api/3/search` and emits one [`Resource`] per
//! issue. Authentication is Basic auth: `email:api_token` base64-encoded —
//! the API token comes from `id.atlassian.com/manage-profile/security/api-tokens`.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "site_url":  "https://mycorp.atlassian.net",
//!   "email":     "ops@mycorp.com",
//!   "api_token": "ATATT...",
//!   "jql":       "ORDER BY updated DESC",
//!   "page_size": 50,
//!   "max_issues": 1000
//! }
//! ```
//!
//! `site_url` must include the scheme; trailing slashes are tolerated.
//! `jql` defaults to `"ORDER BY updated DESC"` if omitted — narrow it
//! with a project filter (e.g. `"project = ENG ORDER BY updated DESC"`)
//! to scope what the connector pulls.
//!
//! # Limitations (v0.4)
//!
//! - Issue body is taken from `fields.description` rendered to plain text
//!   via ADF (Atlassian Document Format) flattening. Macros / inline cards
//!   beyond the basic node types are dropped.
//! - Comments, attachments, and subtasks are not pulled.
//! - Permissions: emitted as `jira:project:<key>` — finer-grained issue
//!   security schemes aren't projected.

use async_trait::async_trait;
use base64::Engine;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Principal, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use time::OffsetDateTime;

const DEFAULT_JQL: &str = "ORDER BY updated DESC";
const DEFAULT_PAGE_SIZE: u32 = 50;
const DEFAULT_MAX_ISSUES: u32 = 1000;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const USER_AGENT: &str = concat!("sealstack-jira/", env!("CARGO_PKG_VERSION"));
const FIELDS: &str = "summary,description,updated,project,issuetype,status";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Site base URL, e.g. `"https://mycorp.atlassian.net"`.
    pub site_url: String,
    /// User email associated with the API token.
    pub email: String,
    /// Personal API token from id.atlassian.com.
    pub api_token: String,
    /// JQL filter. Defaults to `ORDER BY updated DESC`.
    #[serde(default)]
    pub jql: Option<String>,
    #[serde(default)]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub max_issues: Option<u32>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct JiraConnector {
    client: reqwest::Client,
    site_url: String,
    jql: String,
    page_size: u32,
    max_issues: u32,
}

impl JiraConnector {
    /// # Errors
    /// See [`Self::new`].
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("jira connector config: {e}")))?;
        Self::new(config)
    }

    /// # Errors
    /// Returns [`SealStackError::Config`] on missing fields or malformed URL,
    /// [`SealStackError::Backend`] on reqwest client build failure.
    pub fn new(config: Config) -> SealStackResult<Self> {
        if config.site_url.is_empty() {
            return Err(SealStackError::Config(
                "jira connector requires `site_url`".into(),
            ));
        }
        if config.email.is_empty() {
            return Err(SealStackError::Config(
                "jira connector requires `email`".into(),
            ));
        }
        if config.api_token.is_empty() {
            return Err(SealStackError::Config(
                "jira connector requires `api_token`".into(),
            ));
        }
        if !config.site_url.starts_with("http://") && !config.site_url.starts_with("https://") {
            return Err(SealStackError::Config(
                "jira `site_url` must start with http:// or https://".into(),
            ));
        }

        let site_url = config.site_url.trim_end_matches('/').to_string();
        let jql = config
            .jql
            .clone()
            .unwrap_or_else(|| DEFAULT_JQL.to_string());
        let page_size = config.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        let max_issues = config.max_issues.unwrap_or(DEFAULT_MAX_ISSUES);
        let timeout =
            Duration::from_secs(config.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));

        let raw = format!("{}:{}", config.email, config.api_token);
        let auth_b64 = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let auth = format!("Basic {auth_b64}");
        let auth_header = reqwest::header::HeaderValue::from_str(&auth)
            .map_err(|e| SealStackError::Config(format!("jira auth header invalid: {e}")))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, auth_header);
        headers.insert(
            reqwest::header::ACCEPT,
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
            site_url,
            jql,
            page_size,
            max_issues,
        })
    }

    async fn list_issues(&self) -> SealStackResult<Vec<JiraIssue>> {
        let url = format!("{}/rest/api/3/search", self.site_url);
        let mut out: Vec<JiraIssue> = Vec::new();
        let mut start_at: u32 = 0;

        loop {
            if out.len() >= self.max_issues as usize {
                break;
            }
            let resp = self
                .client
                .get(&url)
                .query(&[
                    ("jql", self.jql.as_str()),
                    ("fields", FIELDS),
                    ("startAt", &start_at.to_string()),
                    ("maxResults", &self.page_size.to_string()),
                ])
                .send()
                .await
                .map_err(|e| SealStackError::Backend(format!("jira search: {e}")))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SealStackError::Unauthorized(
                    "jira: credentials rejected".into(),
                ));
            }
            if !status.is_success() {
                return Err(SealStackError::Backend(format!(
                    "jira search: HTTP {status}",
                )));
            }
            let page: SearchResponse = resp
                .json()
                .await
                .map_err(|e| SealStackError::Backend(format!("jira search body: {e}")))?;
            let received = page.issues.len() as u32;
            for issue in page.issues {
                out.push(issue);
                if out.len() >= self.max_issues as usize {
                    break;
                }
            }
            if received == 0 {
                break;
            }
            start_at += received;
            if start_at >= page.total {
                break;
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl Connector for JiraConnector {
    fn name(&self) -> &str {
        "jira"
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
        let key = id.as_str().strip_prefix("jira://issue/").ok_or_else(|| {
            SealStackError::NotFound(format!("id `{id}` is not a jira issue reference"))
        })?;
        let url = format!("{}/rest/api/3/issue/{}", self.site_url, key);
        let resp = self
            .client
            .get(&url)
            .query(&[("fields", FIELDS)])
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("jira issue: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SealStackError::NotFound(format!("jira issue {key}")));
        }
        if !status.is_success() {
            return Err(SealStackError::Backend(format!(
                "jira issue: HTTP {status}",
            )));
        }
        let issue: JiraIssue = resp
            .json()
            .await
            .map_err(|e| SealStackError::Backend(format!("jira issue body: {e}")))?;
        Ok(issue_to_resource(&issue))
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let url = format!("{}/rest/api/3/myself", self.site_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("jira healthcheck: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SealStackError::Unauthorized(
                "jira: credentials rejected".into(),
            ));
        }
        if !resp.status().is_success() {
            return Err(SealStackError::Backend(format!(
                "jira healthcheck: HTTP {}",
                resp.status(),
            )));
        }
        Ok(())
    }
}

fn issue_to_resource(issue: &JiraIssue) -> Resource {
    let fields = &issue.fields;
    let summary = fields.summary.clone().unwrap_or_default();
    let body = fields
        .description
        .as_ref()
        .map(flatten_adf)
        .unwrap_or_default();
    let updated = fields
        .updated
        .as_deref()
        .and_then(parse_jira_datetime)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let project_key = fields
        .project
        .as_ref()
        .map(|p| p.key.clone())
        .unwrap_or_else(|| "unknown".into());
    let mut metadata = serde_json::Map::new();
    metadata.insert("jira_id".into(), Value::String(issue.id.clone()));
    metadata.insert("key".into(), Value::String(issue.key.clone()));
    if let Some(p) = &fields.project {
        metadata.insert("project".into(), Value::String(p.key.clone()));
    }
    if let Some(t) = &fields.issuetype {
        metadata.insert("issuetype".into(), Value::String(t.name.clone()));
    }
    if let Some(s) = &fields.status {
        metadata.insert("status".into(), Value::String(s.name.clone()));
    }
    Resource {
        id: ResourceId::new(format!("jira://issue/{}", issue.key)),
        kind: "issue".into(),
        title: Some(summary),
        body,
        metadata,
        permissions: vec![PermissionPredicate {
            principal: Principal::Group(format!("jira:project:{project_key}")),
            action: "read".into(),
        }],
        source_updated_at: updated,
    }
}

/// Flatten an Atlassian Document Format (ADF) node tree into plain text.
///
/// ADF is a recursive JSON document model. Each node has `type` and may
/// have `content` (children) and `text`. Walking the tree and
/// concatenating leaf text gives a reasonable index body.
fn flatten_adf(node: &Value) -> String {
    let mut out = String::new();
    flatten_adf_into(node, &mut out);
    out
}

fn flatten_adf_into(node: &Value, out: &mut String) {
    if let Some(text) = node.get("text").and_then(Value::as_str) {
        out.push_str(text);
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for c in children {
            flatten_adf_into(c, out);
        }
        // Insert a line break after block-like nodes for readability.
        if matches!(
            node.get("type").and_then(Value::as_str),
            Some(
                "paragraph"
                    | "heading"
                    | "bulletList"
                    | "orderedList"
                    | "listItem"
                    | "codeBlock"
                    | "blockquote"
            )
        ) {
            out.push('\n');
        }
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    issues: Vec<JiraIssue>,
    #[serde(default)]
    total: u32,
}

#[derive(Debug, Deserialize, Clone)]
struct JiraIssue {
    id: String,
    key: String,
    #[serde(default)]
    fields: JiraFields,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct JiraFields {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<Value>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    project: Option<JiraProject>,
    #[serde(default)]
    issuetype: Option<JiraType>,
    #[serde(default)]
    status: Option<JiraStatus>,
}

#[derive(Debug, Deserialize, Clone)]
struct JiraProject {
    key: String,
}

#[derive(Debug, Deserialize, Clone)]
struct JiraType {
    name: String,
}

#[derive(Debug, Deserialize, Clone)]
struct JiraStatus {
    name: String,
}

/// Parse Jira's API datetime: ISO 8601 with timezone offset, e.g.
/// `2024-09-15T14:30:00.000-0700` (no colon in offset).
fn parse_jira_datetime(s: &str) -> Option<OffsetDateTime> {
    // Try RFC 3339 first (matches `+07:00` and `Z`).
    let fmt = time::format_description::well_known::Rfc3339;
    if let Ok(dt) = OffsetDateTime::parse(s, &fmt) {
        return Some(dt);
    }
    // Jira sometimes emits `-0700` without colon — insert one and retry.
    if s.len() >= 5 {
        let (head, tail) = s.split_at(s.len() - 5);
        let bytes = tail.as_bytes();
        if (bytes[0] == b'+' || bytes[0] == b'-') && bytes[1..].iter().all(u8::is_ascii_digit) {
            let fixed = format!(
                "{head}{}{}:{}",
                tail.chars().next()?,
                &tail[1..3],
                &tail[3..5]
            );
            if let Ok(dt) = OffsetDateTime::parse(&fixed, &fmt) {
                return Some(dt);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: Value) -> Value {
        let mut base = serde_json::json!({
            "site_url":  "https://acme.atlassian.net",
            "email":     "ops@acme.com",
            "api_token": "ATATT-x",
        });
        if let Value::Object(m) = extra {
            for (k, v) in m {
                base[k] = v;
            }
        }
        base
    }

    #[test]
    fn config_rejects_missing_token() {
        let json = serde_json::json!({
            "site_url": "https://acme.atlassian.net",
            "email":    "ops@acme.com",
            "api_token": "",
        });
        let err = JiraConnector::from_json(&json).expect_err("empty token rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unknown_field() {
        let json = cfg(serde_json::json!({ "spelunk": true }));
        let err = JiraConnector::from_json(&json).expect_err("unknown rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_non_http_site() {
        let json = serde_json::json!({
            "site_url": "ftp://example.com",
            "email":    "ops@x",
            "api_token": "x",
        });
        let err = JiraConnector::from_json(&json).expect_err("ftp rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_strips_trailing_slash() {
        let json = serde_json::json!({
            "site_url": "https://acme.atlassian.net/",
            "email":    "ops@acme.com",
            "api_token": "x",
        });
        let c = JiraConnector::from_json(&json).expect("builds");
        assert_eq!(c.site_url, "https://acme.atlassian.net");
    }

    #[test]
    fn flatten_adf_concatenates_leaves() {
        let adf = serde_json::json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "Hello "},
                        {"type": "text", "text": "world"},
                    ]
                },
                {
                    "type": "heading",
                    "content": [{"type": "text", "text": "H2"}]
                }
            ]
        });
        let out = flatten_adf(&adf);
        assert_eq!(out, "Hello world\nH2\n");
    }

    #[test]
    fn flatten_adf_handles_empty_doc() {
        let adf = serde_json::json!({ "type": "doc", "content": [] });
        assert_eq!(flatten_adf(&adf), "");
    }

    #[test]
    fn issue_to_resource_emits_project_predicate() {
        let issue = JiraIssue {
            id: "10001".into(),
            key: "ENG-7".into(),
            fields: JiraFields {
                summary: Some("Fix retry".into()),
                description: Some(serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "body"}]
                    }]
                })),
                updated: Some("2024-01-02T03:04:05.000+0000".into()),
                project: Some(JiraProject { key: "ENG".into() }),
                issuetype: Some(JiraType { name: "Bug".into() }),
                status: Some(JiraStatus {
                    name: "In Progress".into(),
                }),
            },
        };
        let r = issue_to_resource(&issue);
        assert_eq!(r.id.as_str(), "jira://issue/ENG-7");
        assert_eq!(r.title.as_deref(), Some("Fix retry"));
        assert_eq!(r.body, "body\n");
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "jira:project:ENG"
        ));
    }

    #[test]
    fn parse_jira_datetime_handles_offset_without_colon() {
        let dt = parse_jira_datetime("2024-01-02T03:04:05.000+0000").expect("parses");
        assert_eq!(dt.year(), 2024);
    }
}
