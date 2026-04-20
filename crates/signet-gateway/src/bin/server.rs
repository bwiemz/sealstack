//! `signet-gateway` binary entry point.
//!
//! Boot sequence:
//!
//! 1. Load gateway config from env.
//! 2. Initialize structured logging.
//! 3. Instantiate vector store and embedder (in-memory dev defaults).
//! 4. Build [`signet_engine::Engine`].
//! 5. Build the connector factory (knows about `local-files`).
//! 6. Hand both to [`signet_gateway::build_app`] and serve.

use std::sync::Arc;

use signet_connector_sdk::Connector;
use signet_embedders::Embedder;
use signet_engine::policy::PolicyEngine;
use signet_engine::rerank::Reranker;
use signet_engine::{Engine, EngineConfig};
use signet_gateway::server::ConnectorFactory;
use signet_vectorstore::VectorStore;
use serde_json::Value;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = signet_gateway::config::Config::from_env();

    tracing_subscriber::registry()
        .with(EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json())
        .init();

    tracing::info!(bind = %config.bind, "starting signet-gateway");

    // ---- Engine boot ------------------------------------------------------
    let engine_config = EngineConfig {
        database_url: config.database_url.clone(),
        ..EngineConfig::test()
    };
    let vector_store: Arc<dyn VectorStore> = dev_vector_store(&config).await?;
    let embedder: Arc<dyn Embedder> = dev_embedder();
    let reranker: Arc<dyn Reranker> = dev_reranker();
    let policy: Arc<dyn PolicyEngine> = dev_policy();
    let engine = Arc::new(
        Engine::new(engine_config, vector_store, embedder, policy, reranker).await?,
    );

    // ---- Connector factory ------------------------------------------------
    let factory: ConnectorFactory = Arc::new(
        |kind: &str, config: &Value| -> anyhow::Result<Arc<dyn Connector>> {
            match kind {
                "local-files" => {
                    let root = config
                        .get("root")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            anyhow::anyhow!("local-files connector requires a `root` string")
                        })?;
                    let c = signet_connector_local_files::LocalFilesConnector::new(root)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    Ok(Arc::new(c))
                }
                "github" => {
                    let cfg = signet_connector_github::GithubConfig::from_json(config)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    Ok(Arc::new(signet_connector_github::GithubConnector::new(cfg)))
                }
                "slack" => {
                    let cfg = signet_connector_slack::SlackConfig::from_json(config)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    Ok(Arc::new(signet_connector_slack::SlackConnector::new(cfg)))
                }
                other => Err(anyhow::anyhow!("unknown connector kind `{other}`")),
            }
        },
    );

    // ---- Serve ------------------------------------------------------------
    let app = signet_gateway::build_app(config.clone(), engine, factory).await?;
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");

    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Dev backend wiring.
// ---------------------------------------------------------------------------

async fn dev_vector_store(
    config: &signet_gateway::config::Config,
) -> anyhow::Result<Arc<dyn VectorStore>> {
    if config.qdrant_url.is_empty() {
        tracing::warn!("using in-process vector store; data is not persisted across restarts");
        Ok(Arc::new(signet_vectorstore::memory::InMemoryStore::default()))
    } else {
        tracing::info!(url = %config.qdrant_url, "connecting to Qdrant");
        let qdrant =
            signet_vectorstore::qdrant::QdrantStore::connect(&config.qdrant_url).await?;
        Ok(Arc::new(qdrant))
    }
}

/// Select the embedder backend at boot from `SIGNET_EMBEDDER`.
///
/// Recognized values:
///
/// * `stub` (default) — deterministic BLAKE3-derived vectors. No network.
/// * `openai` — `OpenAI` `/v1/embeddings`. Reads `OPENAI_API_KEY` and optional
///   `SIGNET_EMBEDDER_MODEL`, `SIGNET_EMBEDDER_ENDPOINT`, `SIGNET_EMBEDDER_DIMS`.
/// * `voyage` — `Voyage` AI embeddings. Reads `VOYAGE_API_KEY` and optional
///   `SIGNET_EMBEDDER_MODEL`.
///
/// On misconfiguration (missing key, unknown model) the function logs an
/// error and falls back to the stub embedder so the process still boots.
fn dev_embedder() -> Arc<dyn Embedder> {
    let kind = std::env::var("SIGNET_EMBEDDER").unwrap_or_else(|_| "stub".into());
    match kind.as_str() {
        "openai" => build_openai_embedder().unwrap_or_else(fallback_stub),
        "voyage" => build_voyage_embedder().unwrap_or_else(fallback_stub),
        "stub" | "" => {
            tracing::warn!("using stub embedder; results will not be semantically meaningful");
            Arc::new(signet_embedders::stub::StubEmbedder::new(64))
        }
        other => {
            tracing::error!(requested = %other, "unknown SIGNET_EMBEDDER; falling back to stub");
            Arc::new(signet_embedders::stub::StubEmbedder::new(64))
        }
    }
}

#[allow(clippy::needless_pass_by_value)] // signature pinned by `Result::unwrap_or_else`
fn fallback_stub(err: anyhow::Error) -> Arc<dyn Embedder> {
    tracing::error!(error = %err, "embedder init failed; falling back to stub");
    Arc::new(signet_embedders::stub::StubEmbedder::new(64))
}

fn build_openai_embedder() -> anyhow::Result<Arc<dyn Embedder>> {
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    let model =
        std::env::var("SIGNET_EMBEDDER_MODEL").unwrap_or_else(|_| "text-embedding-3-small".into());
    let mut e = signet_embedders::openai::OpenAIEmbedder::with_model(api_key, &model)
        .map_err(|e| anyhow::anyhow!("openai embedder: {e}"))?;
    if let Ok(ep) = std::env::var("SIGNET_EMBEDDER_ENDPOINT") {
        e = e.with_endpoint(ep);
    }
    if let Some(dims) = std::env::var("SIGNET_EMBEDDER_DIMS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    {
        e = e.with_dims(dims);
    }
    tracing::info!(backend = "openai", %model, "embedder initialized");
    Ok(Arc::new(e))
}

fn build_voyage_embedder() -> anyhow::Result<Arc<dyn Embedder>> {
    let api_key = std::env::var("VOYAGE_API_KEY").unwrap_or_default();
    let model = std::env::var("SIGNET_EMBEDDER_MODEL").unwrap_or_else(|_| "voyage-3".into());
    let e = signet_embedders::voyage::VoyageEmbedder::with_model(api_key, &model)
        .map_err(|e| anyhow::anyhow!("voyage embedder: {e}"))?;
    tracing::info!(backend = "voyage", %model, "embedder initialized");
    Ok(Arc::new(e))
}

/// Select the policy engine at boot from `SIGNET_POLICY_DIR` + `SIGNET_POLICY_DEFAULT`.
///
/// * Unset / empty `SIGNET_POLICY_DIR` → `AllowAllPolicy`, with a warning. Fine
///   for dev, never for prod.
/// * Directory set → scan it for `<ns>.<schema>.wasm` bundles and evaluate
///   compiled policies. `SIGNET_POLICY_DEFAULT=deny` flips missing-bundle
///   behavior to deny (fail-closed); default is `allow`.
fn dev_policy() -> Arc<dyn PolicyEngine> {
    let dir = std::env::var("SIGNET_POLICY_DIR").unwrap_or_default();
    if dir.is_empty() {
        return signet_engine::policy::default_dev_policy();
    }
    let deny_missing =
        std::env::var("SIGNET_POLICY_DEFAULT").map(|v| v == "deny").unwrap_or(false);
    let result = if deny_missing {
        signet_engine::policy::WasmPolicy::load_from_dir_deny_missing(&dir)
    } else {
        signet_engine::policy::WasmPolicy::load_from_dir(&dir)
    };
    match result {
        Ok(p) => {
            tracing::info!(%dir, deny_missing, "wasm policy engine initialized");
            Arc::new(p)
        }
        Err(e) => {
            tracing::error!(error = %e, %dir, "wasm policy init failed; falling back to AllowAllPolicy");
            signet_engine::policy::default_dev_policy()
        }
    }
}

/// Select the reranker backend at boot from `SIGNET_RERANKER`.
///
/// * `identity` (default) — no reordering, returns candidates unchanged.
/// * `http` — POSTs to `SIGNET_RERANKER_URL` with OpenAI-compatible `/v1/rerank`
///   shape. Reads `SIGNET_RERANKER_MODEL` and optional `SIGNET_RERANKER_API_KEY`.
fn dev_reranker() -> Arc<dyn Reranker> {
    let kind = std::env::var("SIGNET_RERANKER").unwrap_or_else(|_| "identity".into());
    match kind.as_str() {
        "http" => {
            let endpoint = std::env::var("SIGNET_RERANKER_URL").unwrap_or_default();
            if endpoint.is_empty() {
                tracing::error!("SIGNET_RERANKER=http but SIGNET_RERANKER_URL unset; falling back to identity");
                return Arc::new(signet_engine::rerank::IdentityReranker);
            }
            let model = std::env::var("SIGNET_RERANKER_MODEL")
                .unwrap_or_else(|_| "rerank-default".into());
            let api_key = std::env::var("SIGNET_RERANKER_API_KEY").ok();
            tracing::info!(backend = "http", %endpoint, %model, "reranker initialized");
            Arc::new(signet_engine::rerank::HttpReranker {
                endpoint,
                model,
                api_key,
                client: reqwest::Client::new(),
            })
        }
        "identity" | "" => Arc::new(signet_engine::rerank::IdentityReranker),
        other => {
            tracing::error!(requested = %other, "unknown SIGNET_RERANKER; falling back to identity");
            Arc::new(signet_engine::rerank::IdentityReranker)
        }
    }
}
