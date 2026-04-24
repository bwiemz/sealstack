//! Connector SDK.
//!
//! To add a new data source to SealStack, create a crate, depend on this
//! one, and implement the [`Connector`] trait.
//!
//! # Minimum viable connector
//!
//! ```no_run
//! use async_trait::async_trait;
//! use sealstack_connector_sdk::{Connector, Resource, ResourceId, ResourceStream, change_streams};
//! use sealstack_common::SealStackResult;
//!
//! pub struct MyConnector;
//!
//! #[async_trait]
//! impl Connector for MyConnector {
//!     fn name(&self) -> &str { "my-connector" }
//!     fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }
//!
//!     async fn list(&self) -> SealStackResult<ResourceStream> {
//!         // Produce every resource the connector knows about.
//!         Ok(change_streams::resource_stream(vec![]))
//!     }
//!
//!     async fn fetch(&self, _id: &ResourceId) -> SealStackResult<Resource> {
//!         todo!("fetch one resource by id")
//!     }
//! }
//! ```
//!
//! The `ResourceStream` alias hides the boxed-stream type. Most connectors use
//! one of the helper constructors in [`change_streams`].

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod auth;
pub mod change_streams;
pub mod http;
pub mod paginate;
pub mod retry;

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

pub use sealstack_common::{SealStackError, SealStackResult};

// ---------------------------------------------------------------------------
// Stream aliases
// ---------------------------------------------------------------------------

/// Pinned, heap-allocated stream of [`Resource`]s produced by a connector.
pub type ResourceStream = Pin<Box<dyn Stream<Item = Resource> + Send>>;

/// Pinned, heap-allocated stream of [`ChangeEvent`]s for push-based connectors.
pub type ChangeStream = Pin<Box<dyn Stream<Item = ChangeEvent> + Send>>;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Opaque, connector-scoped resource identifier.
///
/// The string is meaningful only within the connector that produced it — e.g.
/// an absolute file path for `local-files`, a `gid://` URL for GitHub, a
/// `thread_ts` for Slack. The engine treats it as a blob.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ResourceId(pub String);

impl ResourceId {
    /// Construct from any stringish value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Borrow the raw string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for ResourceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ResourceId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// One resource returned by a connector.
///
/// The engine's [`Ingestor`](../../sealstack_engine/ingest/index.html) takes a
/// `Resource` plus a [`SchemaMeta`](../../sealstack_engine/schema_registry/struct.SchemaMeta.html),
/// writes the row to Postgres, chunks the body, embeds the chunks, and stores
/// the vectors. The connector's only job is to produce this shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    /// Connector-scoped id.
    pub id: ResourceId,
    /// Logical kind (e.g. `"markdown"`, `"issue"`, `"message"`). Informational.
    pub kind: String,
    /// Optional display title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The text body that becomes the chunked field in the target schema.
    pub body: String,
    /// Extra metadata — arbitrary JSON object, surfaced on every search receipt.
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    /// Source-side permission predicates. Rendered verbatim onto receipts;
    /// v0.1 does not enforce them at retrieval — policy is CSL-declared.
    #[serde(default)]
    pub permissions: Vec<PermissionPredicate>,
    /// Last-modified timestamp from the source system (UTC). Used for
    /// freshness decay and for incremental sync.
    pub source_updated_at: time::OffsetDateTime,
}

impl Resource {
    /// Convenience constructor for tests.
    pub fn stub(id: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: ResourceId::new(id),
            kind: "stub".into(),
            title: None,
            body: body.into(),
            metadata: serde_json::Map::new(),
            permissions: vec![PermissionPredicate::public_read()],
            source_updated_at: time::OffsetDateTime::now_utc(),
        }
    }
}

/// Coarse source-side permission predicate.
///
/// `principal` is a stringly-typed identity reference that the source system
/// understands — e.g. `"user:alice@acme.com"`, `"group:engineering"`, `"*"`.
/// `action` is typically `"read"`, `"write"`, or `"list"`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionPredicate {
    /// The identity string the source assigns access to.
    pub principal: String,
    /// The action the principal may perform.
    pub action: String,
}

impl PermissionPredicate {
    /// Predicate granting `read` access to every principal.
    ///
    /// Used by connectors that index public content or where the source system
    /// does not expose per-resource ACLs.
    #[must_use]
    pub fn public_read() -> Self {
        Self {
            principal: "*".into(),
            action: "read".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Change events (push-based connectors)
// ---------------------------------------------------------------------------

/// One change event from a push-based connector.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeEvent {
    /// Resource was created or modified.
    Upsert(Resource),
    /// Resource was removed.
    Delete(ResourceId),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// The connector trait.
///
/// Implementations must be safe to call concurrently from multiple tasks. The
/// ingest runtime may invoke `list()` once at boot while another task calls
/// `fetch()` on demand; all operations take `&self`.
///
/// Errors fall into [`SealStackError::Backend`] for transient / infrastructural
/// failures and [`SealStackError::Unauthorized`] when the source rejects credentials.
/// Never leak source-side PII through error strings.
#[async_trait]
pub trait Connector: Send + Sync + 'static {
    /// Short, stable identifier. Must match the name in
    /// `sealstack connector add <name>`.
    fn name(&self) -> &str;

    /// Connector implementation version, typically `env!("CARGO_PKG_VERSION")`.
    fn version(&self) -> &str;

    /// Stream every resource the authenticated principal can see.
    ///
    /// Called once per sync cycle. For connectors where the source is large,
    /// implementations should stream lazily rather than buffer the whole set
    /// in memory — the ingest runtime back-pressures naturally.
    async fn list(&self) -> SealStackResult<ResourceStream>;

    /// Fetch one resource by id.
    ///
    /// Used for on-demand refresh (e.g., a webhook notified us of a change)
    /// and for the `sealstack connector refresh <id>` CLI.
    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource>;

    /// Subscribe to change events from the source, if supported.
    ///
    /// Returns `Ok(None)` for pull-only connectors. Pull connectors are driven
    /// by the runtime's poll loop instead.
    async fn subscribe(&self) -> SealStackResult<Option<ChangeStream>> {
        Ok(None)
    }

    /// Healthcheck. Default: succeed unconditionally.
    ///
    /// Connectors that authenticate against a remote service should override
    /// to exercise the credentials path — typically a cheap `whoami`-style
    /// call.
    async fn healthcheck(&self) -> SealStackResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn resource_stream_helper_roundtrips() {
        let v = vec![Resource::stub("a", "body-a"), Resource::stub("b", "body-b")];
        let mut s = change_streams::resource_stream(v);
        let first = s.next().await.unwrap();
        assert_eq!(first.id.as_str(), "a");
        let second = s.next().await.unwrap();
        assert_eq!(second.id.as_str(), "b");
        assert!(s.next().await.is_none());
    }

    #[test]
    fn permission_public_read_round_trips() {
        let p = PermissionPredicate::public_read();
        assert_eq!(p.principal, "*");
        assert_eq!(p.action, "read");
    }

    #[test]
    fn resource_id_display() {
        let id: ResourceId = "abc".into();
        assert_eq!(id.to_string(), "abc");
    }
}
