//! Gmail API connector.
//!
//! Lists messages via `GET /gmail/v1/users/me/messages` and fetches each
//! one's plain-text body via `GET /gmail/v1/users/me/messages/{id}?format=full`.
//! Authentication is OAuth 2.0 bearer — the connector takes a short-lived
//! `access_token` rather than handling Google's OAuth dance itself, because
//! refresh-token rotation belongs in whatever identity provider feeds the
//! gateway (Authentik, Keycloak, Workspace IdP) rather than in a connector.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "access_token": "ya29.xxx",
//!   "query":        "after:2024/01/01",
//!   "page_size":    50,
//!   "max_messages": 500
//! }
//! ```
//!
//! `query` is a Gmail-search-syntax filter applied at the API level so
//! we don't pull every message in a mailbox by accident. Common patterns:
//! `"label:inbox"`, `"is:unread newer_than:30d"`, `"after:2024/01/01 -from:noreply"`.
//!
//! # Limitations (v0.4)
//!
//! - Plain-text body only. HTML alternative parts are flattened to text;
//!   inline images and attachments are dropped.
//! - The token is consumed verbatim — when it expires the connector will
//!   start failing. Rotate by re-registering the connector with a fresh
//!   token (or wire a refresh-token holder in front of the gateway).
//! - Permission predicate: `gmail:user:<email>` derived from the
//!   `From:` header heuristically.

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

const DEFAULT_API_BASE: &str = "https://gmail.googleapis.com";
const DEFAULT_PAGE_SIZE: u32 = 50;
const DEFAULT_MAX_MESSAGES: u32 = 500;
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
const USER_AGENT: &str = concat!("sealstack-gmail/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// OAuth 2.0 access token. Short-lived; refresh externally.
    pub access_token: String,
    /// Optional Gmail search query. Empty = inbox default.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional API base override (for testing).
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub max_messages: Option<u32>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct GmailConnector {
    client: reqwest::Client,
    api_base: String,
    query: Option<String>,
    page_size: u32,
    max_messages: u32,
}

impl GmailConnector {
    /// # Errors
    /// See [`Self::new`].
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("gmail connector config: {e}")))?;
        Self::new(config)
    }

    /// # Errors
    /// Returns [`SealStackError::Config`] on empty token.
    /// Returns [`SealStackError::Backend`] if reqwest client build fails.
    pub fn new(config: Config) -> SealStackResult<Self> {
        if config.access_token.is_empty() {
            return Err(SealStackError::Config(
                "gmail connector requires `access_token`".into(),
            ));
        }
        let api_base = config
            .api_base
            .clone()
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        let page_size = config.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        let max_messages = config.max_messages.unwrap_or(DEFAULT_MAX_MESSAGES);
        let timeout =
            Duration::from_secs(config.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS));

        let auth = format!("Bearer {}", config.access_token);
        let auth_header = reqwest::header::HeaderValue::from_str(&auth).map_err(|e| {
            SealStackError::Config(format!("gmail token has invalid header characters: {e}"))
        })?;

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

        let query = config.query.clone();
        drop(config);
        Ok(Self {
            client,
            api_base,
            query,
            page_size,
            max_messages,
        })
    }

    async fn list_ids(&self) -> SealStackResult<Vec<String>> {
        let url = format!("{}/gmail/v1/users/me/messages", self.api_base);
        let mut out: Vec<String> = Vec::new();
        let mut next_page: Option<String> = None;
        loop {
            if out.len() >= self.max_messages as usize {
                break;
            }
            let page_size = self.page_size.to_string();
            let mut query: Vec<(&str, &str)> = vec![("maxResults", &page_size)];
            if let Some(q) = &self.query {
                query.push(("q", q));
            }
            if let Some(p) = &next_page {
                query.push(("pageToken", p));
            }
            let resp = self
                .client
                .get(&url)
                .query(&query)
                .send()
                .await
                .map_err(|e| SealStackError::Backend(format!("gmail list: {e}")))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SealStackError::Unauthorized(
                    "gmail: access token rejected".into(),
                ));
            }
            if !status.is_success() {
                return Err(SealStackError::Backend(format!(
                    "gmail list: HTTP {status}",
                )));
            }
            let page: ListResponse = resp
                .json()
                .await
                .map_err(|e| SealStackError::Backend(format!("gmail list body: {e}")))?;
            for m in page.messages.unwrap_or_default() {
                out.push(m.id);
                if out.len() >= self.max_messages as usize {
                    break;
                }
            }
            match page.next_page_token {
                Some(t) if !t.is_empty() => next_page = Some(t),
                _ => break,
            }
        }
        Ok(out)
    }

    async fn fetch_message(&self, id: &str) -> SealStackResult<Resource> {
        let url = format!("{}/gmail/v1/users/me/messages/{}", self.api_base, id);
        let resp = self
            .client
            .get(&url)
            .query(&[("format", "full")])
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("gmail message: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SealStackError::NotFound(format!("gmail message {id}")));
        }
        if !status.is_success() {
            return Err(SealStackError::Backend(format!(
                "gmail message: HTTP {status}",
            )));
        }
        let msg: GmailMessage = resp
            .json()
            .await
            .map_err(|e| SealStackError::Backend(format!("gmail message body: {e}")))?;
        Ok(message_to_resource(&msg))
    }
}

#[async_trait]
impl Connector for GmailConnector {
    fn name(&self) -> &str {
        "gmail"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let ids = self.list_ids().await?;
        let mut out: Vec<Resource> = Vec::with_capacity(ids.len());
        for id in ids {
            match self.fetch_message(&id).await {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!(message_id = %id, error = %e, "skipping gmail message"),
            }
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        let mid = id
            .as_str()
            .strip_prefix("gmail://message/")
            .ok_or_else(|| {
                SealStackError::NotFound(format!("id `{id}` is not a gmail message reference"))
            })?;
        self.fetch_message(mid).await
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let url = format!("{}/gmail/v1/users/me/profile", self.api_base);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("gmail healthcheck: {e}")))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SealStackError::Unauthorized(
                "gmail: access token rejected".into(),
            ));
        }
        if !resp.status().is_success() {
            return Err(SealStackError::Backend(format!(
                "gmail healthcheck: HTTP {}",
                resp.status(),
            )));
        }
        Ok(())
    }
}

fn message_to_resource(msg: &GmailMessage) -> Resource {
    let headers = collect_headers(&msg.payload);
    let subject = headers.get("Subject").cloned().unwrap_or_default();
    let from = headers.get("From").cloned().unwrap_or_default();
    let user_id = extract_email(&from).unwrap_or_else(|| "unknown".into());
    let body = extract_text_body(&msg.payload);
    let updated = msg
        .internal_date
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .map(|millis| {
            OffsetDateTime::from_unix_timestamp(millis / 1000)
                .unwrap_or_else(|_| OffsetDateTime::now_utc())
        })
        .unwrap_or_else(OffsetDateTime::now_utc);

    let mut metadata = serde_json::Map::new();
    metadata.insert("gmail_id".into(), Value::String(msg.id.clone()));
    if let Some(t) = &msg.thread_id {
        metadata.insert("thread_id".into(), Value::String(t.clone()));
    }
    if !from.is_empty() {
        metadata.insert("from".into(), Value::String(from.clone()));
    }
    if let Some(to) = headers.get("To") {
        metadata.insert("to".into(), Value::String(to.clone()));
    }

    Resource {
        id: ResourceId::new(format!("gmail://message/{}", msg.id)),
        kind: "email".into(),
        title: Some(subject),
        body,
        metadata,
        permissions: vec![PermissionPredicate {
            principal: Principal::Group(format!("gmail:user:{user_id}")),
            action: "read".into(),
        }],
        source_updated_at: updated,
    }
}

fn collect_headers(payload: &Option<MessagePart>) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(p) = payload {
        for h in &p.headers {
            out.insert(h.name.clone(), h.value.clone());
        }
    }
    out
}

/// Walk the MIME tree looking for a text/plain part; fall back to flattening
/// the first text/html part. Gmail messages are typically multipart with
/// `parts[0]` being text/plain alternative.
fn extract_text_body(payload: &Option<MessagePart>) -> String {
    let Some(p) = payload else {
        return String::new();
    };
    let mut out = String::new();
    walk_parts(p, &mut out);
    out
}

fn walk_parts(part: &MessagePart, out: &mut String) {
    let mime = part.mime_type.as_deref().unwrap_or("");
    if mime.starts_with("text/plain") {
        if let Some(d) = part.body.as_ref().and_then(|b| b.data.as_deref())
            && let Some(decoded) = decode_b64url(d)
        {
            out.push_str(&decoded);
            out.push('\n');
        }
    } else if mime.starts_with("text/html") && out.is_empty() {
        // Only fall back to HTML if no plain has been found yet.
        if let Some(d) = part.body.as_ref().and_then(|b| b.data.as_deref())
            && let Some(decoded) = decode_b64url(d)
        {
            // Strip the very thin set of HTML tags we can without a parser.
            let stripped: String = decoded
                .replace("<br>", "\n")
                .replace("<br/>", "\n")
                .replace("<br />", "\n")
                .replace("</p>", "\n");
            // Drop residual tags.
            let mut in_tag = false;
            for c in stripped.chars() {
                match (in_tag, c) {
                    (false, '<') => in_tag = true,
                    (true, '>') => in_tag = false,
                    (false, _) => out.push(c),
                    (true, _) => {}
                }
            }
            out.push('\n');
        }
    }
    for child in &part.parts {
        walk_parts(child, out);
    }
}

fn decode_b64url(s: &str) -> Option<String> {
    // Normalize away `=` padding (URL_SAFE rejects missing padding, NO_PAD
    // rejects present padding — strip everywhere and use NO_PAD).
    let trimmed = s.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Pull the email address out of a `From:` header value. RFC 5322 syntax
/// is `Display Name <email@host>`, but real-world senders also drop the
/// angle brackets. We take whatever looks like an address.
fn extract_email(value: &str) -> Option<String> {
    if let (Some(lt), Some(gt)) = (value.find('<'), value.rfind('>'))
        && lt < gt
    {
        return Some(value[lt + 1..gt].to_string());
    }
    if value.contains('@') {
        return Some(value.trim().to_string());
    }
    None
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    messages: Option<Vec<MessageRef>>,
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GmailMessage {
    id: String,
    #[serde(rename = "threadId", default)]
    thread_id: Option<String>,
    #[serde(rename = "internalDate", default)]
    internal_date: Option<String>,
    #[serde(default)]
    payload: Option<MessagePart>,
}

#[derive(Debug, Deserialize, Default)]
struct MessagePart {
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
    #[serde(default)]
    headers: Vec<HeaderPair>,
    #[serde(default)]
    body: Option<MessageBody>,
    #[serde(default)]
    parts: Vec<MessagePart>,
}

#[derive(Debug, Deserialize)]
struct HeaderPair {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize, Default)]
struct MessageBody {
    #[serde(default)]
    data: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_empty_token() {
        let v = serde_json::json!({ "access_token": "" });
        let err = GmailConnector::from_json(&v).expect_err("empty token rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unknown_field() {
        let v = serde_json::json!({ "access_token": "x", "spelunk": true });
        let err = GmailConnector::from_json(&v).expect_err("unknown rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_accepts_minimal() {
        let v = serde_json::json!({ "access_token": "ya29.xx" });
        let c = GmailConnector::from_json(&v).expect("builds");
        assert_eq!(c.api_base, DEFAULT_API_BASE);
        assert_eq!(c.page_size, DEFAULT_PAGE_SIZE);
        assert!(c.query.is_none());
    }

    #[test]
    fn extract_email_from_display_name() {
        assert_eq!(
            extract_email("Alice <alice@example.com>").as_deref(),
            Some("alice@example.com"),
        );
        assert_eq!(
            extract_email("bare@example.com").as_deref(),
            Some("bare@example.com"),
        );
        assert_eq!(extract_email("no email here"), None);
    }

    #[test]
    fn decode_b64url_handles_padding_styles() {
        let raw = "hello world";
        let with_padding = base64::engine::general_purpose::URL_SAFE.encode(raw);
        assert_eq!(decode_b64url(&with_padding).as_deref(), Some("hello world"));
        let without_padding = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        assert_eq!(
            decode_b64url(&without_padding).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn message_to_resource_pulls_text_plain_body() {
        let body = base64::engine::general_purpose::URL_SAFE.encode("Hi Bob,\nthanks!");
        let msg = GmailMessage {
            id: "m1".into(),
            thread_id: Some("t1".into()),
            internal_date: Some("1700000000000".into()),
            payload: Some(MessagePart {
                mime_type: Some("multipart/alternative".into()),
                headers: vec![
                    HeaderPair {
                        name: "Subject".into(),
                        value: "Status update".into(),
                    },
                    HeaderPair {
                        name: "From".into(),
                        value: "Alice <alice@acme.com>".into(),
                    },
                    HeaderPair {
                        name: "To".into(),
                        value: "team@acme.com".into(),
                    },
                ],
                body: None,
                parts: vec![MessagePart {
                    mime_type: Some("text/plain".into()),
                    headers: vec![],
                    body: Some(MessageBody { data: Some(body) }),
                    parts: vec![],
                }],
            }),
        };
        let r = message_to_resource(&msg);
        assert_eq!(r.id.as_str(), "gmail://message/m1");
        assert_eq!(r.title.as_deref(), Some("Status update"));
        assert!(r.body.contains("thanks!"));
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "gmail:user:alice@acme.com"
        ));
        assert_eq!(
            r.metadata.get("thread_id").and_then(Value::as_str),
            Some("t1"),
        );
    }

    #[test]
    fn message_to_resource_falls_back_to_html_when_no_plain() {
        let html_body =
            base64::engine::general_purpose::URL_SAFE.encode("<p>Hi <b>Bob</b></p><br>thanks");
        let msg = GmailMessage {
            id: "m2".into(),
            thread_id: None,
            internal_date: None,
            payload: Some(MessagePart {
                mime_type: Some("multipart/alternative".into()),
                headers: vec![],
                body: None,
                parts: vec![MessagePart {
                    mime_type: Some("text/html".into()),
                    headers: vec![],
                    body: Some(MessageBody {
                        data: Some(html_body),
                    }),
                    parts: vec![],
                }],
            }),
        };
        let r = message_to_resource(&msg);
        assert!(r.body.contains("Hi"));
        assert!(r.body.contains("Bob"));
        assert!(!r.body.contains("<b>"));
    }

    #[test]
    fn message_to_resource_handles_empty_payload() {
        let msg = GmailMessage {
            id: "m3".into(),
            thread_id: None,
            internal_date: None,
            payload: None,
        };
        let r = message_to_resource(&msg);
        assert_eq!(r.body, "");
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "gmail:user:unknown"
        ));
    }
}
