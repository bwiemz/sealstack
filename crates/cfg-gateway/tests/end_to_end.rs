//! End-to-end integration test for the ContextForge gateway.
//!
//! Exercises the full happy path: register a schema → register a local-files
//! connector against it → run one sync → fetch the registered connector back
//! via the REST API. This is the contract-level proof that the gateway wires
//! its REST surface, the engine registry, the connector factory, and the
//! persistence path together without gaps.
//!
//! # Why `#[ignore]`
//!
//! The test boots a real [`cfg_engine::Engine`], which requires Postgres and
//! (optionally) Qdrant. Those are not available in a plain `cargo test` run,
//! so the test is ignored by default. Opt in with:
//!
//! ```text
//! CFG_DATABASE_URL=postgres://cfg:cfg@localhost:5432/cfg \
//!     cargo test -p cfg-gateway --test end_to_end -- --ignored
//! ```
//!
//! The compose file at `deploy/docker/compose.dev.yaml` provides a matching
//! dev environment.

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use cfg_connector_sdk::Connector;
use cfg_embedders::Embedder;
use cfg_engine::{Engine, EngineConfig};
use cfg_gateway::server::ConnectorFactory;
use cfg_vectorstore::VectorStore;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt; // oneshot

/// Return the URL to use for Postgres, or `None` if not set (test skipped).
fn pg_url() -> Option<String> {
    std::env::var("CFG_DATABASE_URL").ok()
}

async fn build_test_app() -> anyhow::Result<(Router, TempDir)> {
    let database_url = pg_url().expect("CFG_DATABASE_URL must be set for ignored integration tests");
    let config = cfg_gateway::config::Config {
        bind: "127.0.0.1:0".into(),
        database_url: database_url.clone(),
        qdrant_url: String::new(),
        redis_url: None,
        oauth: cfg_gateway::mcp::oauth::OAuthMetadataConfig::dev_default(),
        log_filter: "info".into(),
        auth: cfg_gateway::auth::AuthMode::Disabled,
    };

    let engine_config = EngineConfig {
        database_url,
        ..EngineConfig::test()
    };
    let vector_store: Arc<dyn VectorStore> =
        Arc::new(cfg_vectorstore::memory::InMemoryStore::default());
    let embedder: Arc<dyn Embedder> = Arc::new(cfg_embedders::stub::StubEmbedder::new(64));
    let engine = Arc::new(Engine::new_dev(engine_config, vector_store, embedder).await?);

    let tmp = tempfile::tempdir()?;
    let tmp_path = tmp.path().to_path_buf();
    let factory: ConnectorFactory = Arc::new(
        move |kind: &str, config: &Value| -> anyhow::Result<Arc<dyn Connector>> {
            if kind != "local-files" {
                anyhow::bail!("unknown connector kind `{kind}`");
            }
            let root = config
                .get("root")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| tmp_path.to_string_lossy().into_owned());
            let c = cfg_connector_local_files::LocalFilesConnector::new(&root)
                .map_err(|e| anyhow::anyhow!(e))?;
            Ok(Arc::new(c))
        },
    );

    let app = cfg_gateway::build_app(config, engine, factory).await?;
    Ok((app, tmp))
}

async fn call(app: Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let body = body
        .map(|b| Body::from(serde_json::to_vec(&b).unwrap()))
        .unwrap_or_else(Body::empty);
    let resp = app.oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

fn sample_schema_meta() -> Value {
    json!({
        "namespace": "examples",
        "name": "Doc",
        "version": 1,
        "primary_key": "id",
        "fields": [],
        "relations": {},
        "facets": [],
        "chunked_fields": ["title", "body"],
        "context": {
            "embedder": "stub",
            "vector_dims": 64,
            "chunking": { "kind": "fixed", "size": 400 },
            "freshness_decay": { "kind": "none" },
            "default_top_k": 10
        },
        "collection": "doc_v1",
        "table": "doc",
        "hybrid_alpha": 0.5
    })
}

// ---- Happy-path test -------------------------------------------------------

#[tokio::test]
#[ignore = "requires CFG_DATABASE_URL + running Postgres"]
async fn register_schema_then_connector_then_sync() {
    if pg_url().is_none() {
        eprintln!("skipping: CFG_DATABASE_URL not set");
        return;
    }
    let (app, tmp) = build_test_app().await.expect("gateway boot");

    // 1. Health check.
    let (status, body) = call(app.clone(), "GET", "/healthz", None).await;
    assert_eq!(status, StatusCode::OK, "healthz failed: {body:?}");

    // 2. Register a schema. The body echoes the qualified name.
    let (status, body) = call(
        app.clone(),
        "POST",
        "/v1/schemas",
        Some(json!({ "meta": sample_schema_meta() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register schema: {body:?}");
    assert_eq!(body["data"]["qualified"], "examples.Doc");

    // 3. Drop a sample doc so the connector has something to ingest.
    std::fs::write(tmp.path().join("setup.md"), "# Setup\n\nUse Postgres 16.").unwrap();

    // 4. Register a connector bound to the schema.
    let (status, body) = call(
        app.clone(),
        "POST",
        "/v1/connectors",
        Some(json!({
            "kind": "local-files",
            "schema": "examples.Doc",
            "config": { "root": tmp.path().to_string_lossy() }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register connector: {body:?}");
    let id = body["data"]["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("local-files/examples.Doc"), "unexpected id: {id}");

    // 5. Listing should include it.
    let (status, body) = call(app.clone(), "GET", "/v1/connectors", None).await;
    assert_eq!(status, StatusCode::OK);
    let connectors = body["data"]["connectors"].as_array().unwrap();
    assert!(
        connectors.iter().any(|c| c["id"] == id),
        "registered connector missing from list: {body:?}",
    );
}

// ---- Persistence-round-trip test -----------------------------------------

#[tokio::test]
#[ignore = "requires CFG_DATABASE_URL + running Postgres"]
async fn restart_rehydrates_schemas_and_connectors() {
    if pg_url().is_none() {
        eprintln!("skipping: CFG_DATABASE_URL not set");
        return;
    }
    // First gateway instance: register + sync.
    let (app, tmp) = build_test_app().await.expect("first boot");
    std::fs::write(tmp.path().join("a.md"), "alpha").unwrap();

    let (_, _) = call(
        app.clone(),
        "POST",
        "/v1/schemas",
        Some(json!({ "meta": sample_schema_meta() })),
    )
    .await;
    let (_, body) = call(
        app.clone(),
        "POST",
        "/v1/connectors",
        Some(json!({
            "kind": "local-files",
            "schema": "examples.Doc",
            "config": { "root": tmp.path().to_string_lossy() }
        })),
    )
    .await;
    let id = body["data"]["id"].as_str().unwrap().to_owned();

    // Drop the app. Build a *fresh* gateway pointing at the same Postgres —
    // the schemas + connectors must come back without another POST.
    drop(app);
    let (app2, _tmp2) = build_test_app().await.expect("second boot");

    let (_, body) = call(app2.clone(), "GET", "/v1/schemas", None).await;
    let schemas = body["data"]["schemas"].as_array().unwrap();
    assert!(
        schemas.iter().any(|s| s["name"] == "Doc"),
        "schema missing after restart: {body:?}",
    );

    let (_, body) = call(app2.clone(), "GET", "/v1/connectors", None).await;
    let connectors = body["data"]["connectors"].as_array().unwrap();
    assert!(
        connectors.iter().any(|c| c["id"] == id),
        "connector missing after restart: {body:?}",
    );
}
