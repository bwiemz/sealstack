//! The ingest runtime orchestrator.
//!
//! Drives registered connectors and feeds their resources into
//! [`sealstack_engine::Engine`].
//!
//! # Three modes
//!
//! 1. **One-shot sync** — `sync_once(binding_id)`. Called by the CLI
//!    (`sealstack connector sync <id>`) and by integration tests. Returns a
//!    [`SyncOutcome`] summarizing what happened.
//! 2. **Periodic background sync** — `start_background()`. Spawns a Tokio task
//!    per binding that has an `interval` configured. Cancellation via a
//!    shared `CancellationToken`.
//! 3. **Subscribe-based streaming** — when a connector returns a `Some(...)`
//!    from `subscribe()`, a dedicated task forwards change events into the
//!    engine as they arrive.
//!
//! # Error handling
//!
//! Per-resource ingestion errors are **logged and counted**, not fatal — one
//! bad resource should not halt a sync. A completely failed connector call
//! (e.g. auth error on `list`) is fatal to that sync but not to the runtime.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::ChangeEvent;
use sealstack_engine::{Engine, api::EngineError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::registry::{ConnectorBinding, ConnectorRegistry};

/// Summary of one sync run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncOutcome {
    /// Binding id that ran.
    pub binding_id: String,
    /// High-level outcome kind.
    pub kind: SyncOutcomeKind,
    /// Resources visited (produced by `list`).
    pub resources_seen: u64,
    /// Resources successfully ingested.
    pub resources_ingested: u64,
    /// Resources the ingestor failed on.
    pub resources_failed: u64,
    /// Chunks written to the vector store.
    pub chunks_written: u64,
    /// Timestamp at which the sync started (UTC).
    pub started_at: OffsetDateTime,
    /// Timestamp at which the sync finished (UTC).
    pub finished_at: OffsetDateTime,
    /// Total elapsed wall time in milliseconds.
    pub elapsed_ms: u64,
    /// Last fatal error, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// High-level result of a sync attempt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncOutcomeKind {
    /// Sync completed; there may still be per-resource failures in the counters.
    Completed,
    /// Listing the connector failed; no resources were ingested.
    FailedList,
    /// Sync was cancelled before completion.
    Cancelled,
    /// Binding not found in the registry.
    NotFound,
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// The ingest runtime.
///
/// Clone-cheap — every field is an `Arc`.
#[derive(Clone)]
pub struct IngestRuntime {
    engine: Arc<Engine>,
    registry: ConnectorRegistry,
    background_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl IngestRuntime {
    /// Construct a runtime backed by the given engine.
    #[must_use]
    pub fn new(engine: Arc<Engine>, registry: ConnectorRegistry) -> Self {
        Self {
            engine,
            registry,
            background_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Access the registry (for CLI list / add operations).
    #[must_use]
    pub fn registry(&self) -> &ConnectorRegistry {
        &self.registry
    }

    /// Register a binding.
    pub fn register(&self, binding: ConnectorBinding) {
        self.registry.register(binding);
    }

    /// Run one full sync cycle for a single binding.
    pub async fn sync_once(&self, binding_id: &str) -> SyncOutcome {
        let started_at = OffsetDateTime::now_utc();
        let mut outcome = SyncOutcome {
            binding_id: binding_id.to_owned(),
            kind: SyncOutcomeKind::Completed,
            resources_seen: 0,
            resources_ingested: 0,
            resources_failed: 0,
            chunks_written: 0,
            started_at,
            finished_at: started_at,
            elapsed_ms: 0,
            error: None,
        };

        let Some(binding) = self.registry.get(binding_id) else {
            outcome.kind = SyncOutcomeKind::NotFound;
            outcome.error = Some(format!("binding `{binding_id}` not registered"));
            outcome.finished_at = OffsetDateTime::now_utc();
            return outcome;
        };

        let instant = std::time::Instant::now();
        let result = self.run_sync(&binding, &mut outcome).await;

        outcome.finished_at = OffsetDateTime::now_utc();
        outcome.elapsed_ms = instant.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

        if let Err(e) = result {
            outcome.error = Some(e.to_string());
            if matches!(outcome.kind, SyncOutcomeKind::Completed) {
                outcome.kind = SyncOutcomeKind::FailedList;
            }
        }

        tracing::info!(
            binding = %binding_id,
            kind = ?outcome.kind,
            seen = outcome.resources_seen,
            ingested = outcome.resources_ingested,
            failed = outcome.resources_failed,
            chunks = outcome.chunks_written,
            ms = outcome.elapsed_ms,
            "sync completed",
        );
        outcome
    }

    /// Drive one binding to completion.
    async fn run_sync(
        &self,
        binding: &ConnectorBinding,
        outcome: &mut SyncOutcome,
    ) -> SealStackResult<()> {
        // Resolve schema early — failing here means the sync can't usefully proceed.
        let meta = self
            .engine
            .registry()
            .get(&binding.target_namespace, &binding.target_schema)
            .map_err(|e| match e {
                EngineError::UnknownSchema { namespace, schema } => {
                    SealStackError::Config(format!(
                        "target schema `{namespace}.{schema}` is not registered with the engine"
                    ))
                }
                other => SealStackError::backend(other),
            })?;

        let mut stream = binding.connector.list().await?;
        let ingestor = self.engine.ingestor();

        while let Some(resource) = stream.next().await {
            outcome.resources_seen += 1;
            match ingestor.ingest(&meta, &binding.tenant, resource).await {
                Ok(res) => {
                    outcome.resources_ingested += 1;
                    outcome.chunks_written += res.chunks_written as u64;
                }
                Err(e) => {
                    outcome.resources_failed += 1;
                    tracing::warn!(
                        binding = %binding.id(),
                        error = %e,
                        "failed to ingest resource; continuing",
                    );
                }
            }
        }

        Ok(())
    }

    /// Start a background poll task per binding that has an `interval`.
    ///
    /// Returns immediately; the tasks live until [`Self::shutdown`] is called.
    pub async fn start_background(&self) {
        let mut handles = self.background_tasks.lock().await;
        for binding in self.registry.list() {
            let Some(interval) = binding.interval else {
                continue;
            };
            let runtime = self.clone();
            let id = binding.id();
            let handle = tokio::spawn(async move {
                runtime.poll_loop(id, interval).await;
            });
            handles.push(handle);
        }

        // Also start subscribe loops for push-based connectors, regardless of interval.
        for binding in self.registry.list() {
            if let Ok(Some(stream)) = binding.connector.subscribe().await {
                let runtime = self.clone();
                let handle = tokio::spawn(async move {
                    runtime.subscribe_loop(binding, stream).await;
                });
                handles.push(handle);
            }
        }
    }

    async fn poll_loop(&self, binding_id: String, interval: Duration) {
        tracing::info!(binding = %binding_id, ?interval, "starting poll loop");
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let outcome = self.sync_once(&binding_id).await;
            if matches!(outcome.kind, SyncOutcomeKind::NotFound) {
                // Binding was removed; exit cleanly.
                tracing::info!(binding = %binding_id, "binding removed; stopping poll loop");
                break;
            }
        }
    }

    async fn subscribe_loop(
        &self,
        binding: ConnectorBinding,
        mut stream: sealstack_connector_sdk::ChangeStream,
    ) {
        let meta = match self
            .engine
            .registry()
            .get(&binding.target_namespace, &binding.target_schema)
        {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, binding = %binding.id(), "subscribe aborted — schema missing");
                return;
            }
        };
        let ingestor = self.engine.ingestor();
        tracing::info!(binding = %binding.id(), "starting subscribe loop");
        while let Some(event) = stream.next().await {
            match event {
                ChangeEvent::Upsert(resource) => {
                    if let Err(e) = ingestor.ingest(&meta, &binding.tenant, resource).await {
                        tracing::warn!(error = %e, binding = %binding.id(), "subscribe upsert failed");
                    }
                }
                ChangeEvent::Delete(_id) => {
                    // v0.1: deletes require mapping source id → engine row id
                    // via sealstack_lineage; plumb that when the lineage queries land.
                    tracing::debug!(binding = %binding.id(), "delete event — not yet wired");
                }
            }
        }
        tracing::info!(binding = %binding.id(), "subscribe stream ended");
    }

    /// Cancel every background task spawned by [`Self::start_background`].
    pub async fn shutdown(&self) {
        let mut handles = self.background_tasks.lock().await;
        for h in handles.drain(..) {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sealstack_connector_sdk::{Connector, Resource, ResourceStream, change_streams};

    struct EmptyConnector;

    #[async_trait]
    impl Connector for EmptyConnector {
        fn name(&self) -> &str {
            "empty"
        }
        fn version(&self) -> &str {
            "0.0.0"
        }
        async fn list(&self) -> SealStackResult<ResourceStream> {
            Ok(change_streams::resource_stream(vec![]))
        }
        async fn fetch(
            &self,
            _id: &sealstack_connector_sdk::ResourceId,
        ) -> SealStackResult<Resource> {
            Err(SealStackError::NotFound("no such resource".into()))
        }
    }

    #[test]
    fn registry_round_trip_by_id() {
        let reg = ConnectorRegistry::new();
        let binding = ConnectorBinding {
            connector: Arc::new(EmptyConnector),
            target_namespace: "demo".into(),
            target_schema: "Doc".into(),
            tenant: String::new(),
            interval: None,
        };
        let id = binding.id();
        reg.register(binding);
        assert!(reg.get(&id).is_some());
        assert!(reg.deregister(&id));
        assert!(reg.get(&id).is_none());
    }

    #[test]
    fn binding_id_format_is_stable() {
        let binding = ConnectorBinding {
            connector: Arc::new(EmptyConnector),
            target_namespace: "acme.docs".into(),
            target_schema: "Doc".into(),
            tenant: String::new(),
            interval: None,
        };
        assert_eq!(binding.id(), "empty/acme.docs.Doc");
    }
}
