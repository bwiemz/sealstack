//! Engine configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Top-level engine configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Postgres connection string.
    pub database_url: String,
    /// Max size of the Postgres connection pool. Default: 32.
    #[serde(default = "default_pg_pool_size")]
    pub pg_pool_size: u32,
    /// Directory containing compiled CSL output (manifests, vector plans, migrations).
    ///
    /// Default: `./out`.
    #[serde(default = "default_compile_dir")]
    pub compile_dir: String,
    /// Retrieval-layer tuning.
    #[serde(default)]
    pub retrieval: RetrievalConfig,
    /// Policy engine tuning.
    #[serde(default)]
    pub policy: PolicyConfig,
    /// Receipt retention policy.
    #[serde(default)]
    pub receipts: ReceiptConfig,
}

impl EngineConfig {
    /// Config for integration tests. Points at the environment's `SIGNET_DATABASE_URL`
    /// or a local default.
    #[must_use]
    pub fn test() -> Self {
        Self {
            database_url: std::env::var("SIGNET_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://signet:signet@localhost:5432/signet".into()),
            pg_pool_size: 4,
            compile_dir: "./out".into(),
            retrieval: RetrievalConfig::default(),
            policy: PolicyConfig::default(),
            receipts: ReceiptConfig::default(),
        }
    }
}

/// Retrieval orchestration config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetrievalConfig {
    /// Default hybrid alpha (1.0 = pure vector, 0.0 = pure BM25). Schemas override
    /// via their CSL `context { hybrid_alpha = ... }` block.
    #[serde(default = "default_hybrid_alpha")]
    pub default_hybrid_alpha: f32,
    /// Candidate set size before reranking.
    #[serde(default = "default_candidate_k")]
    pub candidate_k: usize,
    /// Default top-k if the schema and request both omit one.
    #[serde(default = "default_top_k")]
    pub default_top_k: usize,
    /// Per-retrieval timeout.
    #[serde(default = "default_retrieval_timeout", with = "serde_duration")]
    pub retrieval_timeout: Duration,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            default_hybrid_alpha: default_hybrid_alpha(),
            candidate_k: default_candidate_k(),
            default_top_k: default_top_k(),
            retrieval_timeout: default_retrieval_timeout(),
        }
    }
}

/// Policy-engine config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// If `true`, all policy evaluations allow by default. Use only in
    /// single-tenant development environments; never in production.
    #[serde(default)]
    pub allow_all_stub: bool,
    /// Directory containing compiled policy WASM bundles (`<schema>.wasm`).
    #[serde(default = "default_policy_dir")]
    pub wasm_dir: String,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            allow_all_stub: false,
            wasm_dir: default_policy_dir(),
        }
    }
}

/// Receipts retention config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReceiptConfig {
    /// Days to retain receipts before nightly pruning. Default: 90.
    #[serde(default = "default_receipt_days")]
    pub retention_days: u32,
    /// Whether to sign receipts with an Ed25519 key. Enterprise-only.
    #[serde(default)]
    pub sign: bool,
}

impl Default for ReceiptConfig {
    fn default() -> Self {
        Self {
            retention_days: default_receipt_days(),
            sign: false,
        }
    }
}

// --- defaults ---------------------------------------------------------------

fn default_pg_pool_size() -> u32 {
    32
}
fn default_compile_dir() -> String {
    "./out".into()
}
fn default_hybrid_alpha() -> f32 {
    0.6
}
fn default_candidate_k() -> usize {
    64
}
fn default_top_k() -> usize {
    16
}
fn default_retrieval_timeout() -> Duration {
    Duration::from_secs(10)
}
fn default_policy_dir() -> String {
    "./out/policy".into()
}
fn default_receipt_days() -> u32 {
    90
}

// --- duration serde helper --------------------------------------------------

mod serde_duration {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}
