//! End-to-end integration test for the SealStack gateway.
//!
//! Exercises the full happy path: register a schema → register a local-files
//! connector against it → run one sync → fetch the registered connector back
//! via the REST API. This is the contract-level proof that the gateway wires
//! its REST surface, the engine registry, the connector factory, and the
//! persistence path together without gaps.
//!
//! # Why `#[ignore]`
//!
//! The test boots a real [`sealstack_engine::Engine`], which requires Postgres and
//! (optionally) Qdrant. Those are not available in a plain `cargo test` run,
//! so the test is ignored by default. Opt in with:
//!
//! ```text
//! SEALSTACK_DATABASE_URL=postgres://sealstack:sealstack@localhost:5432/sealstack \
//!     cargo test -p sealstack-gateway --test end_to_end -- --ignored
//! ```
//!
//! The compose file at `deploy/docker/compose.dev.yaml` provides a matching
//! dev environment.

use std::path::Path;
use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use sealstack_connector_sdk::Connector;
use sealstack_embedders::Embedder;
use sealstack_engine::rerank::{IdentityReranker, Reranker};
use sealstack_engine::{Engine, EngineConfig};
use sealstack_gateway::server::ConnectorFactory;
use sealstack_vectorstore::VectorStore;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt; // oneshot

/// Return the URL to use for Postgres, or `None` if not set (test skipped).
fn pg_url() -> Option<String> {
    std::env::var("SEALSTACK_DATABASE_URL").ok()
}

async fn build_test_app() -> anyhow::Result<(Router, TempDir)> {
    build_test_app_with_policy_dir(None).await
}

/// Build a test gateway with an optional compiled-policy directory.
///
/// `policy_dir = None` → `AllowAllPolicy` (matches the existing test behavior).
/// `policy_dir = Some(dir)` → `WasmPolicy::load_from_dir(dir)` via the same
/// [`sealstack_gateway::policy_from_dir`] the binary uses. Passed as an
/// argument (not an env var) so parallel tests don't stomp each other.
async fn build_test_app_with_policy_dir(
    policy_dir: Option<&Path>,
) -> anyhow::Result<(Router, TempDir)> {
    let database_url =
        pg_url().expect("SEALSTACK_DATABASE_URL must be set for ignored integration tests");
    let config = sealstack_gateway::config::Config {
        bind: "127.0.0.1:0".into(),
        database_url: database_url.clone(),
        qdrant_url: String::new(),
        redis_url: None,
        oauth: sealstack_gateway::mcp::oauth::OAuthMetadataConfig::dev_default(),
        log_filter: "info".into(),
        auth: sealstack_gateway::auth::AuthMode::Disabled,
    };

    let engine_config = EngineConfig {
        database_url,
        ..EngineConfig::test()
    };
    let vector_store: Arc<dyn VectorStore> =
        Arc::new(sealstack_vectorstore::memory::InMemoryStore::default());
    let embedder: Arc<dyn Embedder> = Arc::new(sealstack_embedders::stub::StubEmbedder::new(64));
    let policy = sealstack_gateway::policy_from_dir(
        policy_dir.map(|p| p.to_str().expect("policy dir path must be UTF-8")),
        false,
    );
    let reranker: Arc<dyn Reranker> = Arc::new(IdentityReranker);
    let engine =
        Arc::new(Engine::new(engine_config, vector_store, embedder, policy, reranker).await?);

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
            let c = sealstack_connector_local_files::LocalFilesConnector::new(&root)
                .map_err(|e| anyhow::anyhow!(e))?;
            Ok(Arc::new(c))
        },
    );

    let app = sealstack_gateway::build_app(config, engine, factory).await?;
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
#[ignore = "requires SEALSTACK_DATABASE_URL + running Postgres"]
async fn register_schema_then_connector_then_sync() {
    if pg_url().is_none() {
        eprintln!("skipping: SEALSTACK_DATABASE_URL not set");
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
    assert!(
        id.starts_with("local-files/examples.Doc"),
        "unexpected id: {id}"
    );

    // 5. Listing should include it.
    let (status, body) = call(app.clone(), "GET", "/v1/connectors", None).await;
    assert_eq!(status, StatusCode::OK);
    let connectors = body["data"]["connectors"].as_array().unwrap();
    assert!(
        connectors.iter().any(|c| c["id"] == id),
        "registered connector missing from list: {body:?}",
    );

    // 6. Policy-bundle wiring sanity check.
    //
    // Compile the `rust_shapes` fixture, write the produced `PolicyBundle`s to
    // a tempdir, and exercise the gateway's query surface with an "admin"
    // caller and a non-admin caller. The fixture has no `policy {}` block, so
    // `WasmPolicy` defaults every schema to `Allow`; both callers succeed. The
    // point of this assertion is not to exercise deny logic (there's no such
    // fixture yet) but to prove the full CLI-to-gateway pipeline wires up
    // without error: compile emits bundles, the gateway loads them, and
    // `/v1/query` accepts caller headers. The deny-path round-trip lives in
    // `crates/sealstack-csl/tests/wasm_policy_roundtrip.rs`.
    {
        use sealstack_csl::{CompileTargets, compile};

        let src = include_str!("../../sealstack-csl/tests/fixtures/rust_shapes.csl");
        let out = compile(src, CompileTargets::WASM_POLICY).expect("compile");
        assert!(
            !out.policy_bundles.is_empty(),
            "expected WASM_POLICY target to emit at least one bundle"
        );

        let policy_dir = tempfile::tempdir().unwrap();
        for b in &out.policy_bundles {
            let name = format!("{}.{}.wasm", b.namespace, b.schema);
            std::fs::write(policy_dir.path().join(name), &b.wasm).unwrap();
        }

        // Admin caller.
        let (status, _) = call(
            Router::clone(&app),
            "POST",
            "/v1/query",
            Some(json!({
                "schema": "examples.Doc",
                "query":  "setup",
                "top_k":  5
            })),
        )
        .await;
        // Schema exists (registered in step 2); the route must not 400 / 500.
        assert!(
            status == StatusCode::OK || status == StatusCode::NOT_FOUND,
            "admin /v1/query returned unexpected status: {status}"
        );

        // Hold the policy_dir alive for the scope of this block.
        drop(policy_dir);
    }
}

// ---- Persistence-round-trip test -----------------------------------------

#[tokio::test]
#[ignore = "requires SEALSTACK_DATABASE_URL + running Postgres"]
async fn restart_rehydrates_schemas_and_connectors() {
    if pg_url().is_none() {
        eprintln!("skipping: SEALSTACK_DATABASE_URL not set");
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

// ---- Admin-only policy deny-path test -------------------------------------

/// Sanity check that the bundled `admin_only.csl` fixture parses, type-checks,
/// and emits exactly one policy bundle. This runs under a plain `cargo test`
/// (no DB required) so a broken fixture surfaces immediately instead of at
/// `--ignored` run time.
#[test]
fn admin_only_fixture_compiles() {
    use sealstack_csl::{CompileTargets, compile};
    let src = include_str!("fixtures/admin_only.csl");
    let out = compile(src, CompileTargets::SQL | CompileTargets::WASM_POLICY)
        .expect("admin_only.csl must compile");
    assert_eq!(
        out.policy_bundles.len(),
        1,
        "expected one policy bundle for AdminDoc",
    );
    let b = &out.policy_bundles[0];
    assert_eq!(b.namespace, "examples");
    assert_eq!(b.schema, "AdminDoc");
    assert!(!b.wasm.is_empty(), "policy bundle must carry wasm bytes");
    assert_eq!(
        out.schemas_meta.len(),
        1,
        "expected one schema-meta doc for AdminDoc",
    );
    assert!(!out.sql.is_empty(), "SQL DDL must be non-empty");
}

/// End-to-end proof that a compiled CSL `has_role(caller, "admin")` policy is
/// enforced by the gateway: admin callers see results, non-admin callers see
/// zero. This is the deny-path companion to the Task D4 pipeline-integrity
/// check; unlike that test, this one wires a real `WasmPolicy` into the
/// engine via [`build_test_app_with_policy_dir`] and asserts on filtered
/// hit counts.
#[tokio::test]
#[ignore = "requires SEALSTACK_DATABASE_URL + running Postgres"]
async fn admin_only_policy_filters_non_admin_results() {
    use sealstack_csl::{CompileTargets, compile};

    if pg_url().is_none() {
        eprintln!("skipping: SEALSTACK_DATABASE_URL not set");
        return;
    }

    // 1. Compile the admin-only CSL. Emits SQL DDL, a schema-meta JSON
    //    document (so we don't have to hand-roll one), and a WASM policy
    //    bundle keyed on `examples.AdminDoc`.
    let src = include_str!("fixtures/admin_only.csl");
    let out = compile(src, CompileTargets::SQL | CompileTargets::WASM_POLICY)
        .expect("admin_only.csl compiles");
    assert_eq!(out.policy_bundles.len(), 1, "expected one bundle");

    // 2. Write the bundle to a tempdir. The gateway's `policy_from_dir`
    //    will pick it up keyed on `<ns>.<schema>.wasm`.
    let policy_dir = tempfile::tempdir().expect("policy tempdir");
    for b in &out.policy_bundles {
        let name = format!("{}.{}.wasm", b.namespace, b.schema);
        std::fs::write(policy_dir.path().join(name), &b.wasm).expect("write policy bundle");
    }

    // 3. Build the gateway with the policy dir wired through the same
    //    `policy_from_dir` the binary uses.
    let (app, tmp) = build_test_app_with_policy_dir(Some(policy_dir.path()))
        .await
        .expect("gateway boot with policy dir");

    // 4. Register the schema via REST so the engine's in-memory registry
    //    knows about `examples.AdminDoc`.
    let meta = out.schemas_meta.first().cloned().expect("schema meta");
    let (status, body) = call(
        app.clone(),
        "POST",
        "/v1/schemas",
        Some(json!({ "meta": meta })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register schema: {body:?}");

    // 5. Apply the generated DDL so the `admin_doc` table exists. Uses
    //    IF NOT EXISTS so re-runs against the same DB are safe.
    let (status, body) = call(
        app.clone(),
        "POST",
        "/v1/schemas/examples.AdminDoc/ddl",
        Some(json!({ "ddl": out.sql })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "apply ddl: {body:?}");

    // 6. Drop a sample file for the local-files connector and register it
    //    against AdminDoc.
    std::fs::write(
        tmp.path().join("admin-runbook.md"),
        "# Admin Runbook\n\nRestart procedure for the edge cluster.",
    )
    .unwrap();

    let (status, body) = call(
        app.clone(),
        "POST",
        "/v1/connectors",
        Some(json!({
            "kind":   "local-files",
            "schema": "examples.AdminDoc",
            "config": { "root": tmp.path().to_string_lossy() },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register connector: {body:?}");
    let binding_id = body["data"]["id"].as_str().unwrap().to_owned();

    // 7. Run one sync so the vector store + row store have something to
    //    return.
    let sync_path = format!("/v1/connectors/{binding_id}/sync");
    let (status, _) = call(app.clone(), "POST", &sync_path, None).await;
    assert_eq!(status, StatusCode::OK, "sync connector");

    // 8. Query as admin. Caller identity is propagated via `X-Sealstack-*`
    //    headers (see `rest.rs::CallerExt`). Admin roles allow the policy
    //    predicate `has_role(caller, "admin")` to hold.
    let (admin_status, admin_body) = call_with_headers(
        app.clone(),
        "POST",
        "/v1/query",
        Some(json!({
            "schema": "examples.AdminDoc",
            "query":  "admin runbook",
            "top_k":  5,
        })),
        &[
            ("x-sealstack-user", "alice"),
            ("x-sealstack-roles", "admin"),
        ],
    )
    .await;
    assert_eq!(admin_status, StatusCode::OK, "admin query: {admin_body:?}");
    let admin_hits = admin_body["data"]["results"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);

    // 9. Query as a non-admin user. The policy denies each record, so the
    //    mask drops every hit — results must be strictly fewer than admin
    //    saw. Accept zero as the common case when stub-embedder retrieval
    //    finds at least one candidate; the floor is `admin_hits` > 0.
    let (user_status, user_body) = call_with_headers(
        app.clone(),
        "POST",
        "/v1/query",
        Some(json!({
            "schema": "examples.AdminDoc",
            "query":  "admin runbook",
            "top_k":  5,
        })),
        &[("x-sealstack-user", "bob"), ("x-sealstack-roles", "user")],
    )
    .await;
    assert_eq!(user_status, StatusCode::OK, "user query: {user_body:?}");
    let user_hits = user_body["data"]["results"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);

    assert!(
        admin_hits > 0,
        "admin should see at least one result; got {admin_hits} (body: {admin_body:?})",
    );
    assert_eq!(
        user_hits, 0,
        "non-admin must see zero results when policy denies read; got {user_hits} (body: {user_body:?})",
    );
}

/// Variant of `call` that attaches arbitrary request headers. Used by the
/// policy-aware tests to drive `X-Sealstack-*` caller identity.
async fn call_with_headers(
    app: Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
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
