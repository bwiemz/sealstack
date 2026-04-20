//! Axum router composition.
//!
//! `build_app` wires together:
//!
//! * REST routes (from `rest::router`).
//! * MCP transport routes (from `mcp::transport::router`) nested under `/mcp`.
//! * OAuth 2.1 well-known endpoints (from `mcp::oauth::router`).
//! * Tower middleware: trace, cors, compression, timeout.
//!
//! # `AppState` composition
//!
//! The state carries four runtime resources:
//!
//! * `engine` — the concrete [`sealstack_engine::Engine`]. REST handlers call through
//!   this for registry / store / receipts access that the trait-object
//!   [`EngineFacade`] does not expose.
//! * `engine_facade` — trait-object reference to the same engine, used by the
//!   MCP tool handlers for JSON-shaped dispatch.
//! * `ingest` — the [`IngestRuntime`] that drives connectors.
//! * `connector_factory` — a closure that constructs `Arc<dyn Connector>` from
//!   a `(kind, config_json)` pair. Injected at `build_app` time so the gateway
//!   does not need a compile-time dependency on specific connectors.

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, http::Method};
use sealstack_connector_sdk::Connector;
use sealstack_engine::Engine;
use sealstack_engine::facade::EngineFacade;
use sealstack_engine::store::PersistedConnector;
use sealstack_ingest::{ConnectorBinding, ConnectorRegistry, IngestRuntime, SyncOutcome};
use serde_json::Value;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    config::Config,
    mcp::{oauth, registry::ToolRegistry, transport::{self, TransportState}},
    rest,
};

/// Factory closure that constructs concrete connectors from a `(kind, config)` pair.
pub type ConnectorFactory = Arc<
    dyn Fn(&str, &Value) -> anyhow::Result<Arc<dyn Connector>> + Send + Sync + 'static,
>;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    /// Effective config.
    pub config: Arc<Config>,
    /// Tool registry (populated at boot from compiled CSL schemas).
    pub registry: ToolRegistry,
    /// Concrete engine, used by the REST layer for registry / store / receipts.
    pub engine: Arc<Engine>,
    /// Engine facade, used by the MCP tool handlers.
    pub engine_facade: Arc<dyn EngineFacade>,
    /// Ingestion runtime.
    pub ingest: Arc<IngestRuntime>,
    /// Connector factory for `POST /v1/connectors`.
    pub connector_factory: ConnectorFactory,
}

impl AppState {
    /// Run one sync cycle for a registered binding.
    pub async fn sync_connector(&self, binding_id: &str) -> Option<SyncOutcome> {
        self.ingest.registry().get(binding_id)?;
        Some(self.ingest.sync_once(binding_id).await)
    }

    /// Construct a connector via the factory and register it against a target schema.
    pub async fn register_connector(
        &self,
        kind: &str,
        namespace: &str,
        schema: &str,
        config: Value,
    ) -> anyhow::Result<String> {
        let connector = (self.connector_factory)(kind, &config)?;
        let binding = ConnectorBinding {
            connector,
            target_namespace: namespace.to_owned(),
            target_schema: schema.to_owned(),
            tenant: String::new(),
            interval: None,
        };
        let id = binding.id();

        // Persist before registering so a crash between these steps leaves
        // the gateway in a consistent state on restart.
        let persisted = PersistedConnector {
            id: id.clone(),
            kind: kind.to_owned(),
            target_namespace: namespace.to_owned(),
            target_schema: schema.to_owned(),
            tenant: binding.tenant.clone(),
            config,
            interval_secs: binding.interval.map(|d| d.as_secs()),
        };
        if let Err(e) = self.engine.store_handle().put_connector(&persisted).await {
            return Err(anyhow::anyhow!("failed to persist connector: {e}"));
        }

        self.ingest.register(binding);
        Ok(id)
    }

    /// Serializable view of every registered connector binding.
    #[must_use]
    pub fn ingest_bindings(&self) -> Vec<sealstack_ingest::registry::ConnectorBindingInfo> {
        self.ingest.registry().list_info()
    }
}

/// Build the complete HTTP application.
pub async fn build_app(
    config: Config,
    engine: Arc<Engine>,
    connector_factory: ConnectorFactory,
) -> anyhow::Result<Router> {
    // Arc<Engine> coerces to Arc<dyn EngineFacade> because Engine: EngineFacade
    // via the blanket impl. Keeps MCP trait-object dispatch and REST typed
    // access both served by a single Engine instance.
    let engine_facade: Arc<dyn EngineFacade> = engine.clone();
    let ingest = Arc::new(IngestRuntime::new(engine.clone(), ConnectorRegistry::new()));

    let state = AppState {
        config: Arc::new(config.clone()),
        registry: ToolRegistry::new(),
        engine,
        engine_facade,
        ingest,
        connector_factory,
    };

    // Rebuild the MCP tool registry from every schema we know about at boot.
    // Covers both on-disk compiled schemas (via Engine::new's load_from_dir)
    // and anything previously persisted via POST /v1/schemas.
    let schemas = state.engine.registry().iter();
    crate::mcp::bootstrap::register_all(
        &state.registry,
        state.engine_facade.clone(),
        &schemas,
    );

    hydrate_connectors(&state).await;

    let auth_mode = Arc::new(config.auth.clone());
    let mcp_transport = transport::router(TransportState::new(state.registry.clone())).layer(
        axum::middleware::from_fn_with_state(auth_mode, crate::auth::require_bearer),
    );
    let oauth_routes = oauth::router(config.oauth.clone());
    let rest_routes = rest::router().with_state(state.clone());

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_origin(Any)
        .allow_headers(Any);

    let app = Router::new()
        .merge(rest_routes)
        .merge(oauth_routes)
        .nest("/mcp", mcp_transport)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(60),
        ));

    Ok(app)
}

/// Reinstantiate every connector binding stored in `sealstack_connectors`.
///
/// Rows whose `kind` is unknown to the factory (e.g. a connector crate was
/// removed since the row was written) are skipped with a warning rather than
/// aborting the whole boot.
async fn hydrate_connectors(state: &AppState) {
    let rows = match state.engine.store_handle().list_connectors().await {
        Ok(rs) => rs,
        Err(e) => {
            tracing::error!(error = %e, "failed to load connectors from store — previously registered bindings will not run until the condition is resolved");
            return;
        }
    };
    let mut ok = 0;
    let mut skipped = 0;
    for row in rows {
        match (state.connector_factory)(&row.kind, &row.config) {
            Ok(connector) => {
                let binding = ConnectorBinding {
                    connector,
                    target_namespace: row.target_namespace.clone(),
                    target_schema: row.target_schema.clone(),
                    tenant: row.tenant.clone(),
                    interval: row.interval_secs.map(Duration::from_secs),
                };
                state.ingest.register(binding);
                ok += 1;
            }
            Err(e) => {
                tracing::warn!(id = %row.id, kind = %row.kind, error = %e, "skipping unhydrated connector");
                skipped += 1;
            }
        }
    }
    tracing::info!(registered = ok, skipped, "hydrated connectors from store");
}
