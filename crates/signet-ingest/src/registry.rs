//! Connector registry and binding records.
//!
//! A [`ConnectorBinding`] associates one connector instance with one target CSL
//! schema. A connector can be bound to multiple schemas in principle — e.g.
//! `github` writing issues into `Ticket` and pull requests into `PullRequest` —
//! so bindings are keyed by `(connector_name, target_qualified_schema)`.

use std::sync::Arc;

use signet_connector_sdk::Connector;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

/// One binding of a connector to a target schema.
#[derive(Clone)]
pub struct ConnectorBinding {
    /// The connector instance.
    pub connector: Arc<dyn Connector>,
    /// CSL namespace of the target schema (e.g. `"acme.docs"`).
    pub target_namespace: String,
    /// CSL schema name (e.g. `"Doc"`).
    pub target_schema: String,
    /// Tenant identifier data from this connector is stamped with. Empty
    /// string means "default tenant" — single-tenant dev deployments use this.
    pub tenant: String,
    /// Optional sync cadence. When `None`, this binding only runs on demand.
    pub interval: Option<std::time::Duration>,
}

impl ConnectorBinding {
    /// Stable identifier for this binding: `"<connector>/<namespace>.<schema>"`.
    #[must_use]
    pub fn id(&self) -> String {
        format!(
            "{}/{}.{}",
            self.connector.name(),
            self.target_namespace,
            self.target_schema,
        )
    }
}

impl std::fmt::Debug for ConnectorBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorBinding")
            .field("id", &self.id())
            .field("interval", &self.interval)
            .finish()
    }
}

/// Serializable view of a [`ConnectorBinding`] for diagnostics and the
/// `GET /v1/connectors` REST endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectorBindingInfo {
    /// Binding id (`"<connector>/<namespace>.<schema>"`).
    pub id: String,
    /// Connector `name()`.
    pub connector: String,
    /// Connector `version()`.
    pub version: String,
    /// Target schema namespace.
    pub namespace: String,
    /// Target schema.
    pub schema: String,
    /// Tenant this binding writes rows under.
    pub tenant: String,
    /// Background sync interval in seconds (`None` = on-demand only).
    pub interval_secs: Option<u64>,
}

/// Concurrent, in-memory registry of connector bindings.
#[derive(Clone, Default)]
pub struct ConnectorRegistry {
    inner: Arc<DashMap<String, ConnectorBinding>>,
}

impl ConnectorRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a binding. Overwrites any existing binding with the same id.
    pub fn register(&self, binding: ConnectorBinding) {
        let id = binding.id();
        tracing::info!(binding = %id, "registering connector binding");
        self.inner.insert(id, binding);
    }

    /// Remove a binding by id. Returns `true` if the binding existed.
    pub fn deregister(&self, id: &str) -> bool {
        self.inner.remove(id).is_some()
    }

    /// Look up a binding by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<ConnectorBinding> {
        self.inner.get(id).map(|r| r.value().clone())
    }

    /// Enumerate all bindings.
    #[must_use]
    pub fn list(&self) -> Vec<ConnectorBinding> {
        self.inner.iter().map(|r| r.value().clone()).collect()
    }

    /// Enumerate serializable summaries for the REST endpoint.
    #[must_use]
    pub fn list_info(&self) -> Vec<ConnectorBindingInfo> {
        self.inner
            .iter()
            .map(|r| {
                let b = r.value();
                ConnectorBindingInfo {
                    id: b.id(),
                    connector: b.connector.name().to_owned(),
                    version: b.connector.version().to_owned(),
                    namespace: b.target_namespace.clone(),
                    schema: b.target_schema.clone(),
                    tenant: b.tenant.clone(),
                    interval_secs: b.interval.map(|d| d.as_secs()),
                }
            })
            .collect()
    }

    /// Count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
