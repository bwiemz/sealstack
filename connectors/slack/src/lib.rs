//! Slack connector.
//!
//! Pulls channel message history via the Slack Web API. Authentication is a
//! bot token (`xoxb-…`) with `channels:history`, `channels:read`, and
//! `groups:history` scopes as needed. Emits one [`Resource`] per message.
//!
//! # Pagination
//!
//! `conversations.history` pages via `response_metadata.next_cursor`. The
//! connector walks until the cursor is empty. Slack's per-page cap is 1000.
//!
//! # Scope
//!
//! v0.1 reads channels the bot is a member of. Thread replies and file
//! attachments are deferred. DMs and MPIMs are not indexed by default for
//! privacy reasons — enabling them would need an explicit opt-in flag.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

use async_trait::async_trait;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use time::OffsetDateTime;

const SLACK_API: &str = "https://slack.com/api";
const UA: &str = concat!("sealstack-slack/", env!("CARGO_PKG_VERSION"));

/// Slack connector configuration.
#[derive(Clone, Debug)]
pub struct SlackConfig {
    /// Bot token (`xoxb-…`).
    pub token: String,
    /// Optional allow-list of channel ids (e.g. `["C01234"]`). Empty = every
    /// channel the bot is a member of.
    pub channels: Vec<String>,
    /// Cap on messages fetched per channel to bound sync time. Defaults 500.
    pub max_messages_per_channel: u32,
}

impl SlackConfig {
    /// Parse from the binding config JSON shape:
    ///
    /// ```json
    /// { "token": "xoxb-...", "channels": ["C01234"], "max_messages_per_channel": 500 }
    /// ```
    pub fn from_json(v: &serde_json::Value) -> SealStackResult<Self> {
        let token = v
            .get("token")
            .and_then(|x| x.as_str())
            .map(str::to_owned)
            .or_else(|| std::env::var("SLACK_BOT_TOKEN").ok())
            .ok_or_else(|| {
                SealStackError::Config("slack connector requires `token` or SLACK_BOT_TOKEN env".into())
            })?;
        let channels: Vec<String> = v
            .get("channels")
            .and_then(|x| x.as_array())
            .map(|arr| arr.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let max_messages_per_channel = v
            .get("max_messages_per_channel")
            .and_then(|x| x.as_u64())
            .map_or(500, |n| n.min(10_000) as u32);
        Ok(Self {
            token,
            channels,
            max_messages_per_channel,
        })
    }
}

/// The Slack connector.
pub struct SlackConnector {
    client: reqwest::Client,
    config: SlackConfig,
}

impl SlackConnector {
    /// Build a connector. Token is not validated until the first API call.
    #[must_use]
    pub fn new(config: SlackConfig) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .build()
            .expect("reqwest client");
        Self { client, config }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> SealStackResult<T> {
        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.config.token)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("slack request: {e}")))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SealStackError::Unauthorized("slack token rejected".into()));
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SealStackError::Backend(format!("slack {status}: {body}")));
        }
        resp.json::<T>()
            .await
            .map_err(|e| SealStackError::Backend(format!("slack decode: {e}")))
    }

    async fn list_channels(&self) -> SealStackResult<Vec<Channel>> {
        let mut cursor = String::new();
        let mut out = Vec::new();
        loop {
            let url = if cursor.is_empty() {
                format!(
                    "{SLACK_API}/conversations.list?limit=1000&exclude_archived=true&types=public_channel,private_channel",
                )
            } else {
                format!(
                    "{SLACK_API}/conversations.list?limit=1000&exclude_archived=true&types=public_channel,private_channel&cursor={cursor}",
                )
            };
            let resp: ListChannelsResp = self.get_json(&url).await?;
            resp.ok_or_err()?;
            for c in resp.channels.unwrap_or_default() {
                if self.config.channels.is_empty() || self.config.channels.contains(&c.id) {
                    out.push(c);
                }
            }
            match resp.response_metadata.and_then(|m| m.next_cursor) {
                Some(c) if !c.is_empty() => cursor = c,
                _ => break,
            }
        }
        Ok(out)
    }

    async fn list_messages(&self, channel_id: &str) -> SealStackResult<Vec<Message>> {
        let cap = self.config.max_messages_per_channel as usize;
        let mut cursor = String::new();
        let mut out: Vec<Message> = Vec::new();
        while out.len() < cap {
            let want = (cap - out.len()).min(1000);
            let url = if cursor.is_empty() {
                format!(
                    "{SLACK_API}/conversations.history?channel={channel_id}&limit={want}",
                )
            } else {
                format!(
                    "{SLACK_API}/conversations.history?channel={channel_id}&limit={want}&cursor={cursor}",
                )
            };
            let resp: HistoryResp = self.get_json(&url).await?;
            resp.ok_or_err()?;
            out.extend(resp.messages.unwrap_or_default());
            match resp.response_metadata.and_then(|m| m.next_cursor) {
                Some(c) if !c.is_empty() => cursor = c,
                _ => break,
            }
        }
        out.truncate(cap);
        Ok(out)
    }
}

#[async_trait]
impl Connector for SlackConnector {
    fn name(&self) -> &str {
        "slack"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let channels = self.list_channels().await?;
        let mut out: Vec<Resource> = Vec::new();
        for channel in channels {
            let messages = match self.list_messages(&channel.id).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(channel = %channel.id, error = %e, "slack channel fetch failed");
                    continue;
                }
            };
            for msg in messages {
                let Some(ts) = msg.ts.clone() else { continue };
                let body = msg.text.clone().unwrap_or_default();
                if body.is_empty() {
                    continue;
                }
                let updated_at = parse_slack_ts(&ts).unwrap_or_else(OffsetDateTime::now_utc);
                out.push(Resource {
                    id: ResourceId::new(format!("slack://{}/{}", channel.id, ts)),
                    kind: "message".into(),
                    title: Some(channel.name.clone().unwrap_or_else(|| channel.id.clone())),
                    body,
                    metadata: serde_json::Map::from_iter([
                        ("channel".into(), serde_json::Value::String(channel.id.clone())),
                        ("ts".into(), serde_json::Value::String(ts.clone())),
                        (
                            "user".into(),
                            serde_json::Value::String(msg.user.unwrap_or_default()),
                        ),
                    ]),
                    permissions: vec![PermissionPredicate {
                        principal: format!("slack:{}", channel.id),
                        action: "read".into(),
                    }],
                    source_updated_at: updated_at,
                });
            }
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        Err(SealStackError::NotFound(format!(
            "slack fetch not yet implemented for `{id}`",
        )))
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        let url = format!("{SLACK_API}/auth.test");
        let resp: AuthTestResp = self.get_json(&url).await?;
        resp.ok_or_err()
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ListChannelsResp {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    channels: Option<Vec<Channel>>,
    #[serde(default)]
    response_metadata: Option<ResponseMetadata>,
}

#[derive(Deserialize)]
struct HistoryResp {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    messages: Option<Vec<Message>>,
    #[serde(default)]
    response_metadata: Option<ResponseMetadata>,
}

#[derive(Deserialize)]
struct AuthTestResp {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

trait SlackOk {
    fn is_ok(&self) -> bool;
    fn err(&self) -> Option<&str>;
    fn ok_or_err(&self) -> SealStackResult<()> {
        if self.is_ok() {
            Ok(())
        } else {
            Err(SealStackError::Backend(format!(
                "slack api: {}",
                self.err().unwrap_or("unknown")
            )))
        }
    }
}

impl SlackOk for ListChannelsResp {
    fn is_ok(&self) -> bool {
        self.ok
    }
    fn err(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
impl SlackOk for HistoryResp {
    fn is_ok(&self) -> bool {
        self.ok
    }
    fn err(&self) -> Option<&str> {
        self.error.as_deref()
    }
}
impl SlackOk for AuthTestResp {
    fn is_ok(&self) -> bool {
        self.ok
    }
    fn err(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[derive(Deserialize)]
struct ResponseMetadata {
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Channel {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

/// Parse Slack's floating-point Unix-timestamp strings
/// (e.g. `"1712345678.000100"`).
fn parse_slack_ts(ts: &str) -> Option<OffsetDateTime> {
    let secs: f64 = ts.parse().ok()?;
    let whole = secs.trunc() as i128;
    let nanos = (secs.fract() * 1e9) as i128;
    let total = whole * 1_000_000_000 + nanos;
    OffsetDateTime::from_unix_timestamp_nanos(total).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_channels_and_cap() {
        let v = serde_json::json!({
            "token": "xoxb-t",
            "channels": ["C1", "C2"],
            "max_messages_per_channel": 100,
        });
        let c = SlackConfig::from_json(&v).unwrap();
        assert_eq!(c.channels, vec!["C1".to_string(), "C2".to_string()]);
        assert_eq!(c.max_messages_per_channel, 100);
    }

    #[test]
    fn slack_ts_parses() {
        let t = parse_slack_ts("1712345678.000100").unwrap();
        assert_eq!(t.unix_timestamp(), 1_712_345_678);
    }

    #[test]
    fn slack_ok_trait_propagates_error() {
        let resp = AuthTestResp {
            ok: false,
            error: Some("invalid_auth".into()),
        };
        match resp.ok_or_err() {
            Err(SealStackError::Backend(m)) => assert!(m.contains("invalid_auth")),
            other => panic!("expected Backend error, got {other:?}"),
        }
    }
}
