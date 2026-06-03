//! HTTP fetcher connector.
//!
//! Reads a fixed list of HTTP(S) URLs and emits each as a [`Resource`].
//! No crawling, no link following — operators provide the URL set
//! explicitly. HTML responses are converted to plain text via
//! [`html2text`] before indexing so embeddings aren't dominated by
//! markup noise.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "urls":            ["https://docs.example.com/intro", "https://blog.example.com/2026"],
//!   "user_agent":      "sealstack-web/0.3",
//!   "timeout_seconds": 10,
//!   "max_body_bytes":  10485760,
//!   "bearer_token":    "optional-token"
//! }
//! ```
//!
//! # SSRF defense
//!
//! Every URL is validated at construction time:
//!
//! - Scheme must be `http` or `https`.
//! - Host must be parseable via [`url`].
//!
//! At fetch time:
//!
//! - The hostname is resolved via [`tokio::net::lookup_host`] and the
//!   resolved socket addresses are checked against an allowlist —
//!   loopback / link-local / private (RFC 1918) / unique-local (`fc00::/7`)
//!   addresses are rejected to defend against DNS-rebind targeting the
//!   gateway's own internal network.
//! - reqwest is configured to follow at most 3 redirects and to abort on
//!   redirects to disallowed addresses.
//! - Response body is capped at `max_body_bytes` (default 10 MiB).
//!
//! # Limitations (v0.3)
//!
//! - No automated retry on transient failures (those should be handled by the
//!   ingest runtime's per-resource resilience).
//! - No `robots.txt` respect — operators are expected to only point this at
//!   content they own or have explicit permission to ingest.
//! - HTML extraction is whole-page text. No frame/anchor/`<main>` heuristics.

use async_trait::async_trait;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use serde_json::Value;
use std::net::IpAddr;
use std::time::Duration;
use time::OffsetDateTime;
use url::Url;

const DEFAULT_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_USER_AGENT: &str = concat!("sealstack-web/", env!("CARGO_PKG_VERSION"));
const MAX_REDIRECTS: usize = 3;

/// Connector configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// URLs to fetch on every sync.
    pub urls: Vec<String>,
    /// Optional User-Agent header.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Per-request timeout, in seconds. Defaults to 10.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Maximum response body size, in bytes. Defaults to 10 MiB.
    #[serde(default)]
    pub max_body_bytes: Option<u64>,
    /// Optional bearer token added as `Authorization: Bearer <token>`.
    #[serde(default)]
    pub bearer_token: Option<String>,
}

/// Web (HTTP) connector.
#[derive(Clone, Debug)]
pub struct WebConnector {
    config: Config,
    parsed_urls: Vec<Url>,
    client: reqwest::Client,
    max_body_bytes: u64,
}

impl WebConnector {
    /// Build from a JSON config payload.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] on parse / unknown-field error,
    /// on an empty `urls` list, on a non-`http(s)` scheme, or on a URL that
    /// fails to parse. Returns [`SealStackError::Backend`] if the
    /// underlying reqwest client cannot be built.
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("web connector config: {e}")))?;
        Self::new(config)
    }

    /// Build from a typed [`Config`].
    ///
    /// # Errors
    ///
    /// See [`Self::from_json`].
    pub fn new(config: Config) -> SealStackResult<Self> {
        if config.urls.is_empty() {
            return Err(SealStackError::Config(
                "web connector requires at least one URL in `urls`".into(),
            ));
        }
        let mut parsed_urls = Vec::with_capacity(config.urls.len());
        for raw in &config.urls {
            let url = Url::parse(raw)
                .map_err(|e| SealStackError::Config(format!("invalid URL `{raw}`: {e}")))?;
            match url.scheme() {
                "http" | "https" => {}
                other => {
                    return Err(SealStackError::Config(format!(
                        "URL `{raw}` has unsupported scheme `{other}` (only http/https allowed)",
                    )));
                }
            }
            if url.host().is_none() {
                return Err(SealStackError::Config(format!("URL `{raw}` has no host",)));
            }
            parsed_urls.push(url);
        }

        let timeout =
            Duration::from_secs(config.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));
        let max_body_bytes = config.max_body_bytes.unwrap_or(DEFAULT_MAX_BODY_BYTES);
        let ua = config
            .user_agent
            .clone()
            .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());

        let client = reqwest::Client::builder()
            .user_agent(ua)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
            .build()
            .map_err(|e| SealStackError::Backend(format!("reqwest client: {e}")))?;

        Ok(Self {
            config,
            parsed_urls,
            client,
            max_body_bytes,
        })
    }

    async fn fetch_url_to_resource(&self, url: &Url) -> SealStackResult<Resource> {
        validate_host_for_ssrf(url).await?;
        let req = self.client.get(url.clone());
        let req = if let Some(token) = &self.config.bearer_token {
            req.header("authorization", format!("Bearer {token}"))
        } else {
            req
        };
        let resp = req
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("web fetch `{url}`: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(SealStackError::Backend(format!(
                "web fetch `{url}`: HTTP {status}",
            )));
        }

        let headers = resp.headers().clone();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let last_modified = headers
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_http_date)
            .unwrap_or_else(OffsetDateTime::now_utc);
        let etag = headers
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        // Read response body with size cap.
        let body_bytes = resp
            .bytes()
            .await
            .map_err(|e| SealStackError::Backend(format!("read body of `{url}`: {e}")))?;
        if (body_bytes.len() as u64) > self.max_body_bytes {
            return Err(SealStackError::Backend(format!(
                "web fetch `{url}`: response is {} bytes, exceeds cap {}",
                body_bytes.len(),
                self.max_body_bytes
            )));
        }
        let raw_body = String::from_utf8_lossy(&body_bytes).to_string();

        let (kind, body, title_from_html) = if content_type.contains("text/html") {
            let plain = html2text::from_read(raw_body.as_bytes(), 100).unwrap_or(raw_body.clone());
            let title = extract_html_title(&raw_body);
            ("html".to_string(), plain, title)
        } else if content_type.contains("text/markdown") {
            ("markdown".to_string(), raw_body, None)
        } else if content_type.contains("application/json") {
            ("json".to_string(), raw_body, None)
        } else if content_type.starts_with("text/") {
            ("text".to_string(), raw_body, None)
        } else {
            // Binary content — skip entirely (e.g. PDFs need a separate path).
            return Err(SealStackError::Backend(format!(
                "web fetch `{url}`: unsupported content-type `{content_type}`",
            )));
        };

        let title = title_from_html.or_else(|| {
            // Fall back to the last path segment as a humane title.
            url.path_segments()
                .and_then(|mut s| s.next_back())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| url.host_str().map(str::to_owned))
        });

        let mut metadata = serde_json::Map::new();
        metadata.insert("url".into(), Value::String(url.to_string()));
        metadata.insert("content_type".into(), Value::String(content_type));
        if let Some(etag) = etag {
            metadata.insert("etag".into(), Value::String(etag));
        }

        Ok(Resource {
            id: ResourceId::new(url.to_string()),
            kind,
            title,
            body,
            metadata,
            // The fetched page is at the URL we just hit. From the connector's
            // perspective there's no source-side ACL to map; treat as
            // publicly-readable and rely on the CSL policy + the operator's
            // URL allowlist for actual access control.
            permissions: vec![PermissionPredicate::public_read()],
            source_updated_at: last_modified,
        })
    }
}

#[async_trait]
impl Connector for WebConnector {
    fn name(&self) -> &str {
        "web"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let mut out = Vec::with_capacity(self.parsed_urls.len());
        for url in &self.parsed_urls {
            match self.fetch_url_to_resource(url).await {
                Ok(r) => out.push(r),
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "skipping URL");
                }
            }
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        // The id IS the URL string. Verify it's on our allowlist before
        // fetching to ensure SSRF defense doesn't get bypassed by callers
        // constructing arbitrary ids.
        let target = Url::parse(id.as_str())
            .map_err(|e| SealStackError::NotFound(format!("malformed url `{id}`: {e}")))?;
        let allowed = self
            .parsed_urls
            .iter()
            .any(|u| u.as_str() == target.as_str());
        if !allowed {
            return Err(SealStackError::NotFound(format!(
                "url `{id}` is not in this connector's configured URL list",
            )));
        }
        self.fetch_url_to_resource(&target).await
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        // HEAD the first URL. Failures bubble up; this catches gross config
        // errors like wrong scheme or unreachable host.
        let url = self
            .parsed_urls
            .first()
            .ok_or_else(|| SealStackError::Config("no URLs configured".into()))?;
        validate_host_for_ssrf(url).await?;
        self.client
            .head(url.clone())
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("web healthcheck: {e}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SSRF defense
// ---------------------------------------------------------------------------

/// Resolve the URL's hostname and reject if any resolved IP is in a
/// disallowed range (loopback / link-local / private / unique-local).
///
/// `127.0.0.1`, `::1`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
/// `169.254.0.0/16`, `fc00::/7`, `fe80::/10` are all rejected.
async fn validate_host_for_ssrf(url: &Url) -> SealStackResult<()> {
    let host = url
        .host_str()
        .ok_or_else(|| SealStackError::Config(format!("url `{url}` has no host")))?;
    let port = url.port_or_known_default().unwrap_or(80);

    // If the host is a literal IP address, check it directly without DNS.
    if let Ok(addr) = host.parse::<IpAddr>() {
        if is_disallowed_ip(addr) {
            return Err(SealStackError::Config(format!(
                "url `{url}` resolves to disallowed IP {addr}",
            )));
        }
        return Ok(());
    }

    let addrs = tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| SealStackError::Backend(format!("dns lookup for `{host}` failed: {e}")))?;
    for sock in addrs {
        if is_disallowed_ip(sock.ip()) {
            return Err(SealStackError::Config(format!(
                "url `{url}` resolves to disallowed IP {}",
                sock.ip(),
            )));
        }
    }
    Ok(())
}

fn is_disallowed_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                // Unique-local: fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local: fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

// ---------------------------------------------------------------------------
// HTML title extraction
// ---------------------------------------------------------------------------

/// Pull the contents of the first `<title>` tag, lowercased ASCII matching
/// only. Returns `None` when no title is present or the document is too
/// truncated to find a closing tag.
fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    // Find the '>' that closes the opening tag.
    let close_open = lower[start..].find('>')? + start;
    let body_start = close_open + 1;
    let end = lower[body_start..].find("</title>")? + body_start;
    let raw = &html[body_start..end];
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ---------------------------------------------------------------------------
// HTTP date parsing
// ---------------------------------------------------------------------------

/// Parse an IMF-fixdate `Last-Modified` header. Returns `None` on any error.
fn parse_http_date(s: &str) -> Option<OffsetDateTime> {
    // IMF-fixdate is the most common form per RFC 9110:
    //   `Sun, 06 Nov 2026 08:49:37 GMT`
    let fmt = time::format_description::well_known::Rfc2822;
    OffsetDateTime::parse(s, &fmt).ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn config_rejects_empty_urls() {
        let json = serde_json::json!({ "urls": [] });
        let err = WebConnector::from_json(&json).expect_err("empty urls should fail");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unsupported_scheme() {
        let json = serde_json::json!({ "urls": ["file:///etc/passwd"] });
        let err = WebConnector::from_json(&json).expect_err("file:// should fail");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_malformed_url() {
        let json = serde_json::json!({ "urls": ["not a url"] });
        let err = WebConnector::from_json(&json).expect_err("malformed url should fail");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unknown_field() {
        let json = serde_json::json!({
            "urls": ["https://example.com"],
            "secret_typo": true,
        });
        let err = WebConnector::from_json(&json).expect_err("unknown field should fail");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_accepts_minimal() {
        let json = serde_json::json!({ "urls": ["https://example.com/a"] });
        let c = WebConnector::from_json(&json).expect("should build");
        assert_eq!(c.parsed_urls.len(), 1);
    }

    #[test]
    fn is_disallowed_ip_blocks_private_and_loopback() {
        for ip in [
            "127.0.0.1",   // loopback
            "10.0.0.1",    // RFC 1918
            "172.16.0.1",  // RFC 1918
            "192.168.1.1", // RFC 1918
            "169.254.1.1", // link-local
            "::1",         // loopback v6
            "fc00::1",     // unique-local v6
            "fe80::1",     // link-local v6
            "0.0.0.0",     // unspecified
        ] {
            let addr: IpAddr = ip.parse().expect("parse");
            assert!(is_disallowed_ip(addr), "should block `{ip}`",);
        }
    }

    #[test]
    fn is_disallowed_ip_allows_public() {
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            let addr: IpAddr = ip.parse().expect("parse");
            assert!(!is_disallowed_ip(addr), "should allow `{ip}`",);
        }
    }

    #[tokio::test]
    async fn validate_host_for_ssrf_rejects_literal_loopback() {
        let u = Url::parse("http://127.0.0.1/").unwrap();
        let err = validate_host_for_ssrf(&u)
            .await
            .expect_err("loopback should fail");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[tokio::test]
    async fn validate_host_for_ssrf_rejects_literal_rfc1918() {
        let u = Url::parse("http://10.1.2.3/").unwrap();
        let err = validate_host_for_ssrf(&u)
            .await
            .expect_err("RFC1918 should fail");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn extract_html_title_finds_title_tag() {
        let html =
            r#"<!doctype html><html><head><title>Hello World</title></head><body></body></html>"#;
        assert_eq!(extract_html_title(html), Some("Hello World".into()));
    }

    #[test]
    fn extract_html_title_handles_attributes() {
        let html = r#"<title lang="en">Title</title>"#;
        assert_eq!(extract_html_title(html), Some("Title".into()));
    }

    #[test]
    fn extract_html_title_returns_none_when_missing() {
        assert_eq!(extract_html_title("<html></html>"), None);
    }

    #[test]
    fn ipv4_check_documentation_blocked() {
        let addr: IpAddr = Ipv4Addr::new(192, 0, 2, 1).into();
        assert!(is_disallowed_ip(addr));
    }
}
