//! Amazon S3 connector.
//!
//! Lists objects under a bucket prefix via `ListObjectsV2` and emits one
//! [`Resource`] per object. Object bodies are pulled via `GetObject`
//! up to a configurable size cap. The connector uses the AWS SDK's
//! standard credential provider chain: env vars, profile files, IMDS,
//! container metadata — whichever resolves first in the host process.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "bucket":      "my-corp-docs",
//!   "prefix":      "shared/",
//!   "region":      "us-east-1",
//!   "endpoint":    null,
//!   "max_objects": 1000,
//!   "max_body_bytes": 1048576,
//!   "include_globs": ["*.md", "*.txt"]
//! }
//! ```
//!
//! `prefix` scopes the sync to a folder-like subset of the bucket — leave it
//! empty (`""`) to index everything. `endpoint` lets you point the connector
//! at an S3-compatible store (MinIO, Ceph, R2) by supplying the full
//! `https://<endpoint>` URL; standard S3 leaves it `null`.
//!
//! `include_globs` is an optional filter applied to object keys after
//! the prefix match — e.g. only ingest `.md` and `.txt`. Glob syntax
//! supported: `*` (any-but-/) and `?` (single char). For richer pattern
//! matching, exclude objects in the source bucket lifecycle policy
//! instead.
//!
//! # Limitations (v0.4)
//!
//! - Plain object bodies only. Multipart-encoded objects with non-text
//!   content types are still emitted but the body is whatever bytes
//!   come back — there is no PDF / Office extraction.
//! - Object permissions are surfaced as a coarse `s3:bucket:<bucket>`
//!   group predicate. S3 ACLs / bucket-policy grants are not parsed.
//! - No paginated listing past `max_objects` — once the cap is reached,
//!   subsequent syncs may pick different objects depending on
//!   `ListObjectsV2`'s lexical order.

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Principal, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

const DEFAULT_MAX_OBJECTS: u32 = 1000;
const DEFAULT_MAX_BODY_BYTES: usize = 1 << 20; // 1 MiB
const LIST_PAGE_SIZE: i32 = 1000;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Bucket name.
    pub bucket: String,
    /// Optional key prefix to scope the sync.
    #[serde(default)]
    pub prefix: Option<String>,
    /// AWS region. Defaults to the AWS SDK's default resolution (env, profile).
    #[serde(default)]
    pub region: Option<String>,
    /// Override endpoint URL (for S3-compatible stores).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Total object cap per sync.
    #[serde(default)]
    pub max_objects: Option<u32>,
    /// Per-object body cap, bytes. Defaults to 1 MiB.
    #[serde(default)]
    pub max_body_bytes: Option<usize>,
    /// Optional whitelist of glob patterns applied to object keys.
    /// If empty, every object under `prefix` is ingested.
    #[serde(default)]
    pub include_globs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct S3Connector {
    client: Client,
    bucket: String,
    prefix: String,
    max_objects: u32,
    max_body_bytes: usize,
    include_globs: Vec<String>,
}

impl S3Connector {
    /// # Errors
    /// See [`Self::new_with_config`].
    pub async fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("s3 connector config: {e}")))?;
        Self::new_with_config(config).await
    }

    /// Build the connector. This is async because AWS SDK configuration
    /// resolution may hit the metadata service / profile loader; the
    /// gateway factory closure must `block_in_place` or wrap accordingly.
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] if the bucket name is empty or
    /// any glob pattern is invalid.
    pub async fn new_with_config(config: Config) -> SealStackResult<Self> {
        if config.bucket.is_empty() {
            return Err(SealStackError::Config(
                "s3 connector requires a non-empty `bucket`".into(),
            ));
        }
        if !valid_bucket_name(&config.bucket) {
            return Err(SealStackError::Config(format!(
                "s3 `bucket` `{}` does not match AWS naming rules \
                 (lowercase letters, digits, hyphens, dots; 3-63 chars)",
                config.bucket,
            )));
        }
        for g in &config.include_globs {
            if g.is_empty() {
                return Err(SealStackError::Config(
                    "s3 connector: `include_globs` entries must be non-empty".into(),
                ));
            }
        }

        let mut loader = aws_config::defaults(BehaviorVersion::latest());
        if let Some(r) = &config.region {
            loader = loader.region(Region::new(r.clone()));
        }
        let shared = loader.load().await;

        let mut s3_builder = aws_sdk_s3::config::Builder::from(&shared);
        if let Some(ep) = &config.endpoint {
            s3_builder = s3_builder.endpoint_url(ep).force_path_style(true);
        }
        let s3_config = s3_builder.build();
        let client = Client::from_conf(s3_config);

        let prefix = config.prefix.clone().unwrap_or_default();
        let max_objects = config.max_objects.unwrap_or(DEFAULT_MAX_OBJECTS);
        let max_body_bytes = config.max_body_bytes.unwrap_or(DEFAULT_MAX_BODY_BYTES);
        let include_globs = config.include_globs.clone();
        let bucket = config.bucket.clone();
        drop(config);

        Ok(Self {
            client,
            bucket,
            prefix,
            max_objects,
            max_body_bytes,
            include_globs,
        })
    }

    async fn list_keys(&self) -> SealStackResult<Vec<S3ObjectMeta>> {
        let mut out: Vec<S3ObjectMeta> = Vec::new();
        let mut continuation: Option<String> = None;

        loop {
            if out.len() >= self.max_objects as usize {
                break;
            }
            let mut req = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .max_keys(LIST_PAGE_SIZE);
            if !self.prefix.is_empty() {
                req = req.prefix(&self.prefix);
            }
            if let Some(c) = &continuation {
                req = req.continuation_token(c);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| SealStackError::Backend(format!("s3 list_objects_v2: {e}")))?;

            for obj in resp.contents.unwrap_or_default() {
                let Some(key) = obj.key.clone() else { continue };
                if !self.include_globs.is_empty()
                    && !self.include_globs.iter().any(|g| glob_match(g, &key))
                {
                    continue;
                }
                out.push(S3ObjectMeta {
                    key,
                    size: obj.size.unwrap_or(0),
                    last_modified: obj
                        .last_modified
                        .map(|d| d.secs())
                        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok()),
                });
                if out.len() >= self.max_objects as usize {
                    break;
                }
            }
            if !resp.is_truncated.unwrap_or(false) {
                break;
            }
            continuation = resp.next_continuation_token;
            if continuation.is_none() {
                break;
            }
        }
        Ok(out)
    }

    async fn fetch_body(&self, key: &str) -> SealStackResult<String> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("s3 get_object {key}: {e}")))?;

        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| SealStackError::Backend(format!("s3 body collect {key}: {e}")))?
            .into_bytes();
        let take_n = std::cmp::min(self.max_body_bytes, bytes.len());
        let head = &bytes[..take_n];
        Ok(String::from_utf8_lossy(head).into_owned())
    }
}

#[async_trait]
impl Connector for S3Connector {
    fn name(&self) -> &str {
        "s3"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let metas = self.list_keys().await?;
        let mut out: Vec<Resource> = Vec::with_capacity(metas.len());
        for meta in metas {
            let body = match self.fetch_body(&meta.key).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(key = %meta.key, error = %e, "skipping s3 object");
                    continue;
                }
            };
            out.push(meta_to_resource(&self.bucket, &meta, body));
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        let key = id
            .as_str()
            .strip_prefix("s3://")
            .ok_or_else(|| SealStackError::NotFound(format!("id `{id}` is not an s3 reference")))?;
        let (bucket, key) = key.split_once('/').ok_or_else(|| {
            SealStackError::NotFound(format!("id `{id}` missing bucket/key separator"))
        })?;
        if bucket != self.bucket {
            return Err(SealStackError::NotFound(format!(
                "id `{id}` is for bucket `{bucket}`; connector bound to `{}`",
                self.bucket
            )));
        }
        let body = self.fetch_body(key).await?;
        Ok(meta_to_resource(
            &self.bucket,
            &S3ObjectMeta {
                key: key.to_string(),
                size: i64::try_from(body.len()).unwrap_or(0),
                last_modified: Some(OffsetDateTime::now_utc()),
            },
            body,
        ))
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|e| SealStackError::Backend(format!("s3 head_bucket: {e}")))?;
        Ok(())
    }
}

fn meta_to_resource(bucket: &str, meta: &S3ObjectMeta, body: String) -> Resource {
    let mut metadata = serde_json::Map::new();
    metadata.insert("bucket".into(), Value::String(bucket.to_string()));
    metadata.insert("key".into(), Value::String(meta.key.clone()));
    metadata.insert("size".into(), Value::from(meta.size));
    let updated = meta.last_modified.unwrap_or_else(OffsetDateTime::now_utc);
    let title = std::path::Path::new(&meta.key)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned);
    Resource {
        id: ResourceId::new(format!("s3://{bucket}/{}", meta.key)),
        kind: "file".into(),
        title,
        body,
        metadata,
        permissions: vec![PermissionPredicate {
            principal: Principal::Group(format!("s3:bucket:{bucket}")),
            action: "read".into(),
        }],
        source_updated_at: updated,
    }
}

#[derive(Debug, Clone)]
struct S3ObjectMeta {
    key: String,
    size: i64,
    last_modified: Option<OffsetDateTime>,
}

/// S3 bucket naming rules: lowercase letters / digits / hyphens / dots,
/// 3-63 chars, doesn't start or end with `-` or `.`. We don't check
/// every rule (consecutive dots, IP-address-like names) — enough to
/// reject obvious mistakes.
fn valid_bucket_name(name: &str) -> bool {
    let len = name.len();
    if !(3..=63).contains(&len) {
        return false;
    }
    let first = name.as_bytes()[0];
    let last = name.as_bytes()[len - 1];
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// Tiny shell-style glob matcher: `*` matches any-but-`/`, `?` matches
/// one char-but-`/`. Sufficient for `*.md` style filters; not a full
/// fnmatch implementation. For complex patterns push the filtering into
/// the bucket lifecycle / prefix scope.
fn glob_match(pattern: &str, value: &str) -> bool {
    glob_match_impl(pattern.as_bytes(), value.as_bytes())
}

fn glob_match_impl(p: &[u8], v: &[u8]) -> bool {
    match (p.first(), v.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(&b'*'), None) => glob_match_impl(&p[1..], v),
        (Some(&b'*'), Some(&b'/')) => glob_match_impl(&p[1..], v),
        (Some(&b'*'), Some(_)) => glob_match_impl(&p[1..], v) || glob_match_impl(p, &v[1..]),
        (Some(&b'?'), Some(&c)) if c != b'/' => glob_match_impl(&p[1..], &v[1..]),
        (Some(&pc), Some(&vc)) if pc == vc => glob_match_impl(&p[1..], &v[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_empty_bucket() {
        let v = serde_json::json!({ "bucket": "" });
        let err = futures::executor::block_on(S3Connector::from_json(&v))
            .expect_err("empty bucket rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_invalid_bucket() {
        let v = serde_json::json!({ "bucket": "Invalid_BUCKET" });
        let err = futures::executor::block_on(S3Connector::from_json(&v))
            .expect_err("invalid bucket rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_rejects_unknown_field() {
        let v = serde_json::json!({ "bucket": "x", "magic": true });
        let err = futures::executor::block_on(S3Connector::from_json(&v))
            .expect_err("unknown field rejected");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn valid_bucket_name_accepts_normal() {
        assert!(valid_bucket_name("my-corp-docs"));
        assert!(valid_bucket_name("doc.bucket.acme"));
        assert!(valid_bucket_name("abc"));
    }

    #[test]
    fn valid_bucket_name_rejects_short_or_long() {
        assert!(!valid_bucket_name("ab"));
        assert!(!valid_bucket_name(&"x".repeat(64)));
    }

    #[test]
    fn valid_bucket_name_rejects_uppercase_and_underscore() {
        assert!(!valid_bucket_name("MyBucket"));
        assert!(!valid_bucket_name("under_score"));
    }

    #[test]
    fn valid_bucket_name_rejects_dash_endpoints() {
        assert!(!valid_bucket_name("-leading"));
        assert!(!valid_bucket_name("trailing-"));
        assert!(!valid_bucket_name(".leading"));
        assert!(!valid_bucket_name("trailing."));
    }

    #[test]
    fn glob_match_matches_extensions() {
        assert!(glob_match("*.md", "README.md"));
        assert!(glob_match("*.md", "deep.notes.md"));
        assert!(!glob_match("*.md", "README.txt"));
    }

    #[test]
    fn glob_match_question_mark_matches_one_char() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a?c", "a/c"));
    }

    #[test]
    fn glob_match_star_does_not_cross_slash() {
        assert!(!glob_match("*.md", "deep/README.md"));
    }

    #[test]
    fn meta_to_resource_emits_bucket_predicate() {
        let meta = S3ObjectMeta {
            key: "docs/intro.md".into(),
            size: 42,
            last_modified: OffsetDateTime::from_unix_timestamp(1_700_000_000).ok(),
        };
        let r = meta_to_resource("acme-docs", &meta, "body".into());
        assert_eq!(r.id.as_str(), "s3://acme-docs/docs/intro.md");
        assert_eq!(r.title.as_deref(), Some("intro.md"));
        assert!(matches!(
            &r.permissions[0].principal,
            Principal::Group(g) if g == "s3:bucket:acme-docs"
        ));
        assert_eq!(r.metadata.get("size").and_then(Value::as_i64), Some(42));
    }
}
