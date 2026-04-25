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

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{BodyCursorPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use time::OffsetDateTime;

const SLACK_API: &str = "https://slack.com/api";
const UA_SUFFIX: &str = concat!("sealstack-slack/", env!("CARGO_PKG_VERSION"));

/// Slack connector configuration (non-secret fields only).
#[derive(Clone, Debug)]
pub struct SlackConfig {
    /// Optional allow-list of channel ids (e.g. `["C01234"]`). Empty = every
    /// channel the bot is a member of.
    pub channels: Vec<String>,
    /// Cap on messages fetched per channel to bound sync time. Defaults 500.
    pub max_messages_per_channel: u32,
}

impl SlackConfig {
    /// Parse non-secret fields from the binding config JSON shape:
    ///
    /// ```json
    /// { "channels": ["C01234"], "max_messages_per_channel": 500 }
    /// ```
    #[must_use]
    pub fn from_json(v: &serde_json::Value) -> Self {
        let channels: Vec<String> = v
            .get("channels")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let max_messages_per_channel = v
            .get("max_messages_per_channel")
            .and_then(serde_json::Value::as_u64)
            .map_or(500, |n| n.min(10_000) as u32);
        Self {
            channels,
            max_messages_per_channel,
        }
    }
}

/// The Slack connector.
#[derive(Debug)]
pub struct SlackConnector {
    http: Arc<HttpClient>,
    config: SlackConfig,
    /// Base URL for the Slack Web API.
    ///
    /// Defaults to `https://slack.com/api`. Tests can override this via the
    /// `api_base` field in the config JSON to point at a wiremock server.
    api_base: String,
}

impl SlackConnector {
    /// Build a connector from a JSON config value.
    ///
    /// Token precedence:
    /// - Config `token` field present → use it (warns if `SLACK_BOT_TOKEN` is
    ///   also set).
    /// - Config absent + `SLACK_BOT_TOKEN` env var present and non-empty → use
    ///   env var.
    /// - Both absent or empty → returns `SealStackError::Config`.
    ///
    /// Token is not validated against the API until the first call.
    pub fn from_json(v: &serde_json::Value) -> SealStackResult<Self> {
        let token = match (
            v.get("token").and_then(|x| x.as_str()),
            std::env::var("SLACK_BOT_TOKEN").ok(),
        ) {
            (Some(t), env_present) => {
                if env_present.is_some() {
                    tracing::warn!("slack: config `token` set; SLACK_BOT_TOKEN env ignored");
                }
                t.to_owned()
            }
            (None, Some(env)) if !env.is_empty() => env,
            _ => {
                return Err(SealStackError::Config(
                    "slack connector requires `token` in config or SLACK_BOT_TOKEN env".into(),
                ));
            }
        };

        let credential = Arc::new(StaticToken::new(token));
        let http = Arc::new(
            HttpClient::new(credential, RetryPolicy::default())?.with_user_agent_suffix(UA_SUFFIX),
        );
        let config = SlackConfig::from_json(v);
        let api_base = v
            .get("api_base")
            .and_then(|x| x.as_str())
            .unwrap_or(SLACK_API)
            .trim_end_matches('/')
            .to_owned();
        Ok(Self {
            http,
            config,
            api_base,
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> SealStackResult<T> {
        let rb = self.http.get(url);
        let resp = self.http.send(rb).await?;
        resp.json::<T>().await
    }

    async fn list_channels(&self) -> SealStackResult<Vec<Channel>> {
        let url = format!("{}/conversations.list", self.api_base);
        let pg = BodyCursorPaginator::<Channel, _, _, _>::new(
            move |c: &HttpClient, cursor: Option<&str>| {
                let mut rb = c.get(&url).query(&[
                    ("limit", "1000"),
                    ("exclude_archived", "true"),
                    ("types", "public_channel,private_channel"),
                ]);
                if let Some(cur) = cursor {
                    rb = rb.query(&[("cursor", cur)]);
                }
                rb
            },
            |v: &serde_json::Value| {
                if !v
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                    return Err(SealStackError::backend(format!("slack api: {err}")));
                }
                let arr = v
                    .get("channels")
                    .and_then(|a| a.as_array())
                    .ok_or_else(|| SealStackError::backend("missing channels"))?;
                arr.iter()
                    .map(|x| {
                        serde_json::from_value::<Channel>(x.clone())
                            .map_err(|e| SealStackError::backend(format!("{e}")))
                    })
                    .collect()
            },
            |v: &serde_json::Value| {
                v.get("response_metadata")
                    .and_then(|m| m.get("next_cursor"))
                    .and_then(|c| c.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            },
        );

        let allowed = self.config.channels.clone();
        let mut stream = paginate(pg, Arc::clone(&self.http));
        let mut out: Vec<Channel> = Vec::new();
        while let Some(item) = stream.next().await {
            let ch = item?;
            if allowed.is_empty() || allowed.contains(&ch.id) {
                out.push(ch);
            }
        }
        Ok(out)
    }

    async fn list_messages(&self, channel_id: &str) -> SealStackResult<Vec<Message>> {
        let cap = self.config.max_messages_per_channel as usize;
        let url = format!("{}/conversations.history", self.api_base);
        let channel_id_owned = channel_id.to_owned();
        let pg = BodyCursorPaginator::<Message, _, _, _>::new(
            move |c: &HttpClient, cursor: Option<&str>| {
                let mut rb = c
                    .get(&url)
                    .query(&[("channel", channel_id_owned.as_str()), ("limit", "1000")]);
                if let Some(cur) = cursor {
                    rb = rb.query(&[("cursor", cur)]);
                }
                rb
            },
            |v: &serde_json::Value| {
                if !v
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                    return Err(SealStackError::backend(format!("slack api: {err}")));
                }
                let arr = v
                    .get("messages")
                    .and_then(|a| a.as_array())
                    .ok_or_else(|| SealStackError::backend("missing messages"))?;
                arr.iter()
                    .map(|x| {
                        serde_json::from_value::<Message>(x.clone())
                            .map_err(|e| SealStackError::backend(format!("{e}")))
                    })
                    .collect()
            },
            |v: &serde_json::Value| {
                v.get("response_metadata")
                    .and_then(|m| m.get("next_cursor"))
                    .and_then(|c| c.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            },
        );

        let mut stream = paginate(pg, Arc::clone(&self.http)).take(cap);
        let mut out: Vec<Message> = Vec::new();
        while let Some(item) = stream.next().await {
            out.push(item?);
        }
        Ok(out)
    }
}

#[async_trait]
impl Connector for SlackConnector {
    fn name(&self) -> &'static str {
        "slack"
    }

    fn version(&self) -> &'static str {
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
                        (
                            "channel".into(),
                            serde_json::Value::String(channel.id.clone()),
                        ),
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
        let url = format!("{}/auth.test", self.api_base);
        let resp: AuthTestResp = self.get_json(&url).await?;
        resp.ok_or_err()
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

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

impl SlackOk for AuthTestResp {
    fn is_ok(&self) -> bool {
        self.ok
    }
    fn err(&self) -> Option<&str> {
        self.error.as_deref()
    }
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
    #[allow(clippy::cast_possible_truncation)]
    let whole = secs.trunc() as i128;
    #[allow(clippy::cast_possible_truncation)]
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
        let c = SlackConfig::from_json(&v);
        assert_eq!(c.channels, vec!["C1".to_string(), "C2".to_string()]);
        assert_eq!(c.max_messages_per_channel, 100);
    }

    #[test]
    fn connector_from_json_with_token() {
        let v = serde_json::json!({ "token": "xoxb-test" });
        let conn = SlackConnector::from_json(&v).unwrap();
        assert_eq!(conn.name(), "slack");
    }

    #[test]
    fn connector_from_json_missing_token_errors() {
        // This test only works when SLACK_BOT_TOKEN is unset. Skip if the var
        // is present in the environment (CI may set it).
        if std::env::var("SLACK_BOT_TOKEN").is_ok() {
            return;
        }
        let v = serde_json::json!({});
        match SlackConnector::from_json(&v) {
            Err(SealStackError::Config(_)) => {}
            other => panic!("expected Config error, got {other:?}"),
        }
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
