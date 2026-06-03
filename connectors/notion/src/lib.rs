//! Notion API connector.
//!
//! Lists pages via `POST /v1/search`, flattens each page's top-level block
//! tree into plain text via `GET /v1/blocks/{id}/children`, and emits one
//! [`Resource`] per page. Deep block-tree traversal (synced blocks, child
//! pages, toggle children) is deferred — v0.3 indexes the top level only,
//! which covers typical doc/wiki pages.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "token":          "secret_xxx",
//!   "page_size":      50,
//!   "max_pages":      500,
//!   "max_blocks_per_page": 200
//! }
//! ```
//!
//! `token` is the internal-integration secret beginning with `secret_`.
//! Public-integration OAuth is out of scope for v0.3.
//!
//! # Limitations (v0.3)
//!
//! - Top-level blocks only. Toggled/synced/child-page content isn't followed.
//! - Database rows are listed alongside pages but their property values are
//!   only summarized into the title; full row projection lands in v0.4.
//! - Rate-limit handling relies on the SDK retry-shim in `runtime.rs`. The
//!   per-request `Notion-Version` header is set to a known-stable date.

use async_trait::async_trait;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Principal, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use time::OffsetDateTime;

/// Pinned Notion API version. Update only when downstream tests confirm
/// the new revision works.
const NOTION_VERSION: &str = "2022-06-28";
const DEFAULT_API_BASE: &str = "https://api.notion.com";
const DEFAULT_PAGE_SIZE: u32 = 50;
const DEFAULT_MAX_PAGES: u32 = 500;
const DEFAULT_MAX_BLOCKS_PER_PAGE: u32 = 200;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const USER_AGENT: &str = concat!("sealstack-notion/", env!("CARGO_PKG_VERSION"));

/// Connector configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Internal-integration secret token (`secret_...`).
    pub token: String,
    /// Optional API base URL — defaults to `https://api.notion.com`. Override
    /// for testing against a local mock server.
    #[serde(default)]
    pub api_base: Option<String>,
    /// Per-page-size for the search endpoint.
    #[serde(default)]
    pub page_size: Option<u32>,
    /// Cap on total pages enumerated per sync.
    #[serde(default)]
    pub max_pages: Option<u32>,
    /// Cap on blocks fetched per page.
    #[serde(default)]
    pub max_blocks_per_page: Option<u32>,
    /// Per-request timeout, seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// Notion connector.
#[derive(Clone, Debug)]
pub struct NotionConnector {
    client: reqwest::Client,
    api_base: String,
    page_size: u32,
    max_pages: u32,
    max_blocks_per_page: u32,
}

impl NotionConnector {
    /// Build from a JSON config payload.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] on parse / unknown-field error or
    /// a missing/malformed token. Returns [`SealStackError::Backend`] if the
    /// underlying reqwest client cannot be built.
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("notion connector config: {e}")))?;
        Self::new(config)
    }

    /// Build from a typed [`Config`].
    ///
    /// # Errors
    ///
    /// See [`Self::from_json`].
    pub fn new(config: Config) -> SealStackResult<Self> {
        if config.token.is_empty() {
            return Err(SealStackError::Config(
                "notion connector requires a non-empty `token`".into(),
            ));
        }

        let api_base = config
            .api_base
            .clone()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        let page_size = config.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        let max_pages = config.max_pages.unwrap_or(DEFAULT_MAX_PAGES);
        let max_blocks_per_page = config
            .max_blocks_per_page
            .unwrap_or(DEFAULT_MAX_BLOCKS_PER_PAGE);
        let timeout =
            Duration::from_secs(config.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Notion-Version",
            reqwest::header::HeaderValue::from_static(NOTION_VERSION),
        );
        let auth = format!("Bearer {}", config.token);
        let auth_header = reqwest::header::HeaderValue::from_str(&auth).map_err(|e| {
            SealStackError::Config(format!("token has invalid header characters: {e}"))
        })?;
        headers.insert(reqwest::header::AUTHORIZATION, auth_header);

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .default_headers(headers)
            .build()
            .map_err(|e| SealStackError::Backend(format!("reqwest client: {e}")))?;

        // `config` is consumed solely to derive these fields; we drop it to
        // avoid carrying the bearer token around in the struct longer than
        // necessary (the auth header is already baked into `client`'s
        // default headers above).
        drop(config);
        Ok(Self {
            client,
            api_base,
            page_size,
            max_pages,
            max_blocks_per_page,
        })
    }

    /// Walk `/v1/search` until we've collected up to `max_pages` page results.
    async fn list_pages(&self) -> SealStackResult<Vec<NotionPage>> {
        let url = format!("{}/v1/search", self.api_base);
        let mut out: Vec<NotionPage> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            if out.len() >= self.max_pages as usize {
                break;
            }
            let mut body = json!({
                "filter":   { "value": "page", "property": "object" },
                "page_size": self.page_size,
            });
            if let Some(c) = &cursor {
                body["start_cursor"] = Value::String(c.clone());
            }

            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| SealStackError::Backend(format!("notion search: {e}")))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SealStackError::Unauthorized(
                    "notion: token rejected".into(),
                ));
            }
            if !status.is_success() {
                return Err(SealStackError::Backend(format!(
                    "notion search: HTTP {status}",
                )));
            }
            let page: SearchResponse = resp
                .json()
                .await
                .map_err(|e| SealStackError::Backend(format!("notion search body: {e}")))?;

            for r in page.results {
                if r.object == "page" {
                    out.push(r);
                }
                if out.len() >= self.max_pages as usize {
                    break;
                }
            }

            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Pull the top-level children blocks of a page and flatten their text.
    async fn page_body(&self, page_id: &str) -> SealStackResult<String> {
        let url = format!(
            "{}/v1/blocks/{}/children?page_size={}",
            self.api_base, page_id, self.max_blocks_per_page,
        );
        let resp =
            self.client.get(&url).send().await.map_err(|e| {
                SealStackError::Backend(format!("notion blocks for {page_id}: {e}"))
            })?;
        if !resp.status().is_success() {
            return Err(SealStackError::Backend(format!(
                "notion blocks for {page_id}: HTTP {}",
                resp.status(),
            )));
        }
        let body: BlockChildrenResponse = resp
            .json()
            .await
            .map_err(|e| SealStackError::Backend(format!("notion blocks body: {e}")))?;
        let mut out = String::new();
        for block in body.results {
            extract_block_text(&block, &mut out);
        }
        Ok(out)
    }
}

#[async_trait]
impl Connector for NotionConnector {
    fn name(&self) -> &str {
        "notion"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let pages = self.list_pages().await?;
        let mut out: Vec<Resource> = Vec::with_capacity(pages.len());

        for page in pages {
            let title = extract_page_title(&page).unwrap_or_else(|| page.id.clone());
            let updated = page
                .last_edited_time
                .as_deref()
                .and_then(parse_iso8601)
                .unwrap_or_else(OffsetDateTime::now_utc);
            let body = match self.page_body(&page.id).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(page_id = %page.id, error = %e, "skipping page");
                    continue;
                }
            };

            let workspace = workspace_id_from_url(page.url.as_deref().unwrap_or(""))
                .unwrap_or_else(|| "unknown".into());
            let mut metadata = serde_json::Map::new();
            metadata.insert("notion_id".into(), Value::String(page.id.clone()));
            if let Some(u) = &page.url {
                metadata.insert("url".into(), Value::String(u.clone()));
            }
            metadata.insert("archived".into(), Value::Bool(page.archived));

            out.push(Resource {
                id: ResourceId::new(format!("notion://page/{}", page.id)),
                kind: "page".into(),
                title: Some(title),
                body,
                metadata,
                permissions: vec![PermissionPredicate {
                    principal: Principal::Group(format!("notion:workspace:{workspace}")),
                    action: "read".into(),
                }],
                source_updated_at: updated,
            });
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        let page_id = id.as_str().strip_prefix("notion://page/").ok_or_else(|| {
            SealStackError::NotFound(format!("id `{id}` is not a notion page reference"))
        })?;

        let url = format!("{}/v1/pages/{}", self.api_base, page_id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("notion page: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SealStackError::NotFound(format!("notion page {page_id}")));
        }
        if !status.is_success() {
            return Err(SealStackError::Backend(format!(
                "notion page: HTTP {status}",
            )));
        }
        let page: NotionPage = resp
            .json()
            .await
            .map_err(|e| SealStackError::Backend(format!("notion page body: {e}")))?;
        let title = extract_page_title(&page).unwrap_or_else(|| page.id.clone());
        let updated = page
            .last_edited_time
            .as_deref()
            .and_then(parse_iso8601)
            .unwrap_or_else(OffsetDateTime::now_utc);
        let body = self.page_body(&page.id).await?;
        let workspace = workspace_id_from_url(page.url.as_deref().unwrap_or(""))
            .unwrap_or_else(|| "unknown".into());

        let mut metadata = serde_json::Map::new();
        metadata.insert("notion_id".into(), Value::String(page.id.clone()));
        if let Some(u) = &page.url {
            metadata.insert("url".into(), Value::String(u.clone()));
        }
        metadata.insert("archived".into(), Value::Bool(page.archived));

        Ok(Resource {
            id: id.clone(),
            kind: "page".into(),
            title: Some(title),
            body,
            metadata,
            permissions: vec![PermissionPredicate {
                principal: Principal::Group(format!("notion:workspace:{workspace}")),
                action: "read".into(),
            }],
            source_updated_at: updated,
        })
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let url = format!("{}/v1/users/me", self.api_base);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("notion healthcheck: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SealStackError::Unauthorized(
                "notion: token rejected".into(),
            ));
        }
        if !status.is_success() {
            return Err(SealStackError::Backend(format!(
                "notion healthcheck: HTTP {status}",
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<NotionPage>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct NotionPage {
    id: String,
    #[serde(default)]
    object: String,
    #[serde(default)]
    last_edited_time: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    properties: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct BlockChildrenResponse {
    #[serde(default)]
    results: Vec<Value>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pull the first plain_text out of a `title` property bag. Page schema
/// varies — sometimes the property is literally `"title"`, sometimes the
/// database column name. Take whichever bag has a `title` array first.
fn extract_page_title(page: &NotionPage) -> Option<String> {
    for (_, prop) in &page.properties {
        if let Some(title_arr) = prop.get("title").and_then(Value::as_array) {
            let combined: String = title_arr
                .iter()
                .filter_map(|t| t.get("plain_text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if !combined.is_empty() {
                return Some(combined);
            }
        }
    }
    None
}

/// Flatten one block's text content into `out`. Recognizes paragraph,
/// heading_1/2/3, bulleted_list_item, numbered_list_item, quote, callout,
/// to_do, and code blocks. Other types are skipped.
fn extract_block_text(block: &Value, out: &mut String) {
    let Some(ty) = block.get("type").and_then(Value::as_str) else {
        return;
    };
    let payload = match block.get(ty) {
        Some(v) => v,
        None => return,
    };

    let rich = payload
        .get("rich_text")
        .or_else(|| payload.get("text"))
        .and_then(Value::as_array);
    if let Some(items) = rich {
        for t in items {
            if let Some(s) = t.get("plain_text").and_then(Value::as_str) {
                out.push_str(s);
            }
        }
        // Newline separator between blocks for readable chunking.
        out.push('\n');
    }
}

/// Parse an ISO 8601 / RFC 3339 string from Notion's API. Notion emits
/// timestamps like `2023-08-09T22:00:00.000Z`.
fn parse_iso8601(s: &str) -> Option<OffsetDateTime> {
    let fmt = time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(s, &fmt).ok()
}

/// Extract a workspace/group identifier from a Notion URL.
///
/// Notion URLs look like `https://www.notion.so/Acme-Workspace-<id>` or
/// `https://www.notion.so/<workspace-slug>/<page-slug-id>`. Use the URL
/// path's first segment as a coarse workspace identifier; not perfect but
/// sufficient for receipt-level lineage.
fn workspace_id_from_url(url: &str) -> Option<String> {
    let trimmed = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let mut parts = trimmed.split('/');
    parts.next()?; // host
    let first = parts.next()?;
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_empty_token() {
        let json = serde_json::json!({ "token": "" });
        let err = NotionConnector::from_json(&json).expect_err("empty token rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unknown_field() {
        let json = serde_json::json!({
            "token": "secret_x",
            "deep_dive": true,
        });
        let err = NotionConnector::from_json(&json).expect_err("unknown field rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_accepts_minimal() {
        let json = serde_json::json!({ "token": "secret_x" });
        let c = NotionConnector::from_json(&json).expect("should build");
        assert_eq!(c.api_base, DEFAULT_API_BASE);
        assert_eq!(c.page_size, DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn extract_page_title_combines_plain_text_segments() {
        let page = NotionPage {
            id: "p".into(),
            object: "page".into(),
            last_edited_time: None,
            url: None,
            archived: false,
            properties: serde_json::Map::from_iter([(
                "Name".into(),
                serde_json::json!({
                    "title": [
                        {"plain_text": "Hello"},
                        {"plain_text": " World"},
                    ]
                }),
            )]),
        };
        assert_eq!(extract_page_title(&page).as_deref(), Some("Hello World"));
    }

    #[test]
    fn extract_block_text_handles_paragraph() {
        let block = serde_json::json!({
            "type": "paragraph",
            "paragraph": {
                "rich_text": [
                    {"plain_text": "alpha "},
                    {"plain_text": "beta"},
                ],
            }
        });
        let mut out = String::new();
        extract_block_text(&block, &mut out);
        assert_eq!(out, "alpha beta\n");
    }

    #[test]
    fn extract_block_text_handles_heading() {
        let block = serde_json::json!({
            "type": "heading_2",
            "heading_2": {
                "rich_text": [{"plain_text": "Title"}],
            }
        });
        let mut out = String::new();
        extract_block_text(&block, &mut out);
        assert_eq!(out, "Title\n");
    }

    #[test]
    fn extract_block_text_skips_unknown_types() {
        let block = serde_json::json!({ "type": "image", "image": {"file": {}} });
        let mut out = String::new();
        extract_block_text(&block, &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn workspace_id_from_url_extracts_first_segment() {
        assert_eq!(
            workspace_id_from_url("https://www.notion.so/acme-team/Hello-abc123"),
            Some("acme-team".into()),
        );
        assert_eq!(
            workspace_id_from_url("https://acme.notion.site/Doc-xyz"),
            Some("Doc-xyz".into()),
        );
        assert_eq!(workspace_id_from_url(""), None);
    }

    #[test]
    fn parse_iso8601_handles_notion_format() {
        let dt = parse_iso8601("2024-09-15T14:30:00.000Z").expect("parses");
        assert_eq!(dt.year(), 2024);
    }
}
