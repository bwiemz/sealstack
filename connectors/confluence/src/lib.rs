//! Confluence Cloud connector.
//!
//! Pulls pages via `GET /wiki/rest/api/content?type=page&expand=body.storage,version,space`
//! and emits one [`Resource`] per page. Body is the storage-format HTML
//! flattened to plain text via `html2text`.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "site_url":  "https://mycorp.atlassian.net",
//!   "email":     "ops@mycorp.com",
//!   "api_token": "ATATT...",
//!   "space_key": "DOCS",
//!   "page_size": 25,
//!   "max_pages": 500
//! }
//! ```
//!
//! `space_key` scopes pulls to a single Confluence space; omit it to pull
//! across every space the API token can see (use carefully — Confluence
//! installs can be huge).
//!
//! # Limitations (v0.4)
//!
//! - Pages only — blog posts, attachments, comments are not pulled.
//! - Page restrictions (view/edit) are surfaced as a coarse
//!   `confluence:space:<key>` predicate; per-page restriction sets are
//!   deferred.

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

const DEFAULT_PAGE_SIZE: u32 = 25;
const DEFAULT_MAX_PAGES: u32 = 500;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const USER_AGENT: &str = concat!("sealstack-confluence/", env!("CARGO_PKG_VERSION"));
const EXPAND: &str = "body.storage,version,space";
const HTML_WIDTH: usize = 100;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub site_url: String,
    pub email: String,
    pub api_token: String,
    #[serde(default)]
    pub space_key: Option<String>,
    #[serde(default)]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub max_pages: Option<u32>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ConfluenceConnector {
    client: reqwest::Client,
    site_url: String,
    space_key: Option<String>,
    page_size: u32,
    max_pages: u32,
}

impl ConfluenceConnector {
    /// # Errors
    /// See [`Self::new`].
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("confluence connector config: {e}")))?;
        Self::new(config)
    }

    /// # Errors
    /// Returns [`SealStackError::Config`] on missing/invalid fields or
    /// [`SealStackError::Backend`] on client build failure.
    pub fn new(config: Config) -> SealStackResult<Self> {
        if config.site_url.is_empty() {
            return Err(SealStackError::Config(
                "confluence connector requires `site_url`".into(),
            ));
        }
        if config.email.is_empty() {
            return Err(SealStackError::Config(
                "confluence connector requires `email`".into(),
            ));
        }
        if config.api_token.is_empty() {
            return Err(SealStackError::Config(
                "confluence connector requires `api_token`".into(),
            ));
        }
        if !config.site_url.starts_with("http://") && !config.site_url.starts_with("https://") {
            return Err(SealStackError::Config(
                "confluence `site_url` must start with http:// or https://".into(),
            ));
        }
        if let Some(k) = &config.space_key {
            if !valid_space_key(k) {
                return Err(SealStackError::Config(format!(
                    "confluence `space_key` must be alphanumeric; got `{k}`"
                )));
            }
        }

        let site_url = config.site_url.trim_end_matches('/').to_string();
        let page_size = config.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        let max_pages = config.max_pages.unwrap_or(DEFAULT_MAX_PAGES);
        let timeout =
            Duration::from_secs(config.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));

        let raw = format!("{}:{}", config.email, config.api_token);
        let auth_b64 = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let auth = format!("Basic {auth_b64}");
        let auth_header = reqwest::header::HeaderValue::from_str(&auth)
            .map_err(|e| SealStackError::Config(format!("confluence auth header invalid: {e}")))?;

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

        let space_key = config.space_key.clone();
        drop(config);
        Ok(Self {
            client,
            site_url,
            space_key,
            page_size,
            max_pages,
        })
    }

    async fn list_pages(&self) -> SealStackResult<Vec<ConfluencePage>> {
        let url = format!("{}/wiki/rest/api/content", self.site_url);
        let mut out: Vec<ConfluencePage> = Vec::new();
        let mut start: u32 = 0;

        loop {
            if out.len() >= self.max_pages as usize {
                break;
            }
            let limit = self.page_size.to_string();
            let start_str = start.to_string();
            let mut query: Vec<(&str, &str)> = vec![
                ("type", "page"),
                ("expand", EXPAND),
                ("start", &start_str),
                ("limit", &limit),
            ];
            if let Some(k) = &self.space_key {
                query.push(("spaceKey", k));
            }

            let resp = self
                .client
                .get(&url)
                .query(&query)
                .send()
                .await
                .map_err(|e| SealStackError::Backend(format!("confluence list: {e}")))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SealStackError::Unauthorized(
                    "confluence: credentials rejected".into(),
                ));
            }
            if !status.is_success() {
                return Err(SealStackError::Backend(format!(
                    "confluence list: HTTP {status}",
                )));
            }
            let page: ListResponse = resp
                .json()
                .await
                .map_err(|e| SealStackError::Backend(format!("confluence list body: {e}")))?;
            let received = page.results.len() as u32;
            for r in page.results {
                out.push(r);
                if out.len() >= self.max_pages as usize {
                    break;
                }
            }
            if received < self.page_size {
                break;
            }
            start += received;
        }
        Ok(out)
    }
}

#[async_trait]
impl Connector for ConfluenceConnector {
    fn name(&self) -> &str {
        "confluence"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let pages = self.list_pages().await?;
        let mut out: Vec<Resource> = Vec::with_capacity(pages.len());
        for page in pages {
            out.push(page_to_resource(&self.site_url, &page));
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        let page_id = id
            .as_str()
            .strip_prefix("confluence://page/")
            .ok_or_else(|| {
                SealStackError::NotFound(format!("id `{id}` is not a confluence page reference"))
            })?;
        let url = format!("{}/wiki/rest/api/content/{}", self.site_url, page_id);
        let resp = self
            .client
            .get(&url)
            .query(&[("expand", EXPAND)])
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("confluence page: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SealStackError::NotFound(format!(
                "confluence page {page_id}"
            )));
        }
        if !status.is_success() {
            return Err(SealStackError::Backend(format!(
                "confluence page: HTTP {status}",
            )));
        }
        let page: ConfluencePage = resp
            .json()
            .await
            .map_err(|e| SealStackError::Backend(format!("confluence page body: {e}")))?;
        Ok(page_to_resource(&self.site_url, &page))
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let url = format!("{}/wiki/rest/api/space", self.site_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("limit", "1")])
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("confluence healthcheck: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SealStackError::Unauthorized(
                "confluence: credentials rejected".into(),
            ));
        }
        if !resp.status().is_success() {
            return Err(SealStackError::Backend(format!(
                "confluence healthcheck: HTTP {}",
                resp.status(),
            )));
        }
        Ok(())
    }
}

fn page_to_resource(site_url: &str, page: &ConfluencePage) -> Resource {
    let body = page
        .body
        .as_ref()
        .and_then(|b| b.storage.as_ref())
        .and_then(|s| s.value.as_deref())
        .map(|raw| {
            html2text::from_read(raw.as_bytes(), HTML_WIDTH).unwrap_or_else(|_| raw.to_string())
        })
        .unwrap_or_default();
    let updated = page
        .version
        .as_ref()
        .and_then(|v| v.when.as_deref())
        .and_then(parse_iso8601)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let space_key = page
        .space
        .as_ref()
        .map(|s| s.key.clone())
        .unwrap_or_else(|| "unknown".into());

    let mut metadata = serde_json::Map::new();
    metadata.insert("confluence_id".into(), Value::String(page.id.clone()));
    metadata.insert("space".into(), Value::String(space_key.clone()));
    if let Some(v) = &page.version {
        if let Some(n) = v.number {
            metadata.insert("version".into(), Value::from(n));
        }
    }
    let url = format!("{site_url}/wiki/spaces/{space_key}/pages/{}", page.id);
    metadata.insert("url".into(), Value::String(url));

    Resource {
        id: ResourceId::new(format!("confluence://page/{}", page.id)),
        kind: "page".into(),
        title: Some(page.title.clone()),
        body,
        metadata,
        permissions: vec![PermissionPredicate {
            principal: Principal::Group(format!("confluence:space:{space_key}")),
            action: "read".into(),
        }],
        source_updated_at: updated,
    }
}

fn valid_space_key(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    results: Vec<ConfluencePage>,
}

#[derive(Debug, Deserialize, Clone)]
struct ConfluencePage {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: Option<ConfluenceBody>,
    #[serde(default)]
    version: Option<ConfluenceVersion>,
    #[serde(default)]
    space: Option<ConfluenceSpace>,
}

#[derive(Debug, Deserialize, Clone)]
struct ConfluenceBody {
    #[serde(default)]
    storage: Option<ConfluenceStorage>,
}

#[derive(Debug, Deserialize, Clone)]
struct ConfluenceStorage {
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ConfluenceVersion {
    #[serde(default)]
    number: Option<u32>,
    #[serde(default)]
    when: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ConfluenceSpace {
    key: String,
}

fn parse_iso8601(s: &str) -> Option<OffsetDateTime> {
    let fmt = time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(s, &fmt).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_cfg() -> Value {
        serde_json::json!({
            "site_url":  "https://acme.atlassian.net",
            "email":     "ops@acme.com",
            "api_token": "x",
        })
    }

    #[test]
    fn config_accepts_minimal() {
        let c = ConfluenceConnector::from_json(&base_cfg()).expect("builds");
        assert_eq!(c.site_url, "https://acme.atlassian.net");
        assert_eq!(c.page_size, DEFAULT_PAGE_SIZE);
        assert!(c.space_key.is_none());
    }

    #[test]
    fn config_rejects_unknown_field() {
        let mut v = base_cfg();
        v["surprise"] = Value::Bool(true);
        let err = ConfluenceConnector::from_json(&v).expect_err("unknown rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_bad_space_key() {
        let mut v = base_cfg();
        v["space_key"] = Value::String("with space".into());
        let err = ConfluenceConnector::from_json(&v).expect_err("bad space rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_accepts_valid_space_key() {
        let mut v = base_cfg();
        v["space_key"] = Value::String("DOCS42".into());
        let c = ConfluenceConnector::from_json(&v).expect("builds");
        assert_eq!(c.space_key.as_deref(), Some("DOCS42"));
    }

    #[test]
    fn config_rejects_non_http_site() {
        let mut v = base_cfg();
        v["site_url"] = Value::String("file:///etc/passwd".into());
        let err = ConfluenceConnector::from_json(&v).expect_err("rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn page_to_resource_flattens_storage_html() {
        let page = ConfluencePage {
            id: "42".into(),
            title: "Welcome".into(),
            body: Some(ConfluenceBody {
                storage: Some(ConfluenceStorage {
                    value: Some("<p>Hello <strong>world</strong>.</p><p>Para two.</p>".into()),
                }),
            }),
            version: Some(ConfluenceVersion {
                number: Some(3),
                when: Some("2024-09-15T14:30:00.000Z".into()),
            }),
            space: Some(ConfluenceSpace { key: "DOCS".into() }),
        };
        let r = page_to_resource("https://acme.atlassian.net", &page);
        assert_eq!(r.id.as_str(), "confluence://page/42");
        assert_eq!(r.title.as_deref(), Some("Welcome"));
        assert!(r.body.contains("Hello"));
        assert!(r.body.contains("Para two"));
        assert_eq!(
            r.metadata.get("space").and_then(Value::as_str),
            Some("DOCS")
        );
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "confluence:space:DOCS"
        ));
    }

    #[test]
    fn page_to_resource_handles_missing_fields() {
        let page = ConfluencePage {
            id: "1".into(),
            title: "T".into(),
            body: None,
            version: None,
            space: None,
        };
        let r = page_to_resource("https://acme.atlassian.net", &page);
        assert_eq!(r.body, "");
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "confluence:space:unknown",
        ));
    }
}
