//! Walk `contracts/fixtures/` and validate each fixture against the
//! api-types wire types.
//!
//! The Phase 1 stub assumed Phase 4 would boot a gateway and *capture*
//! request/response pairs. That requires Postgres on the host running
//! the binary (see `crates/sealstack-gateway/tests/end_to_end.rs`),
//! which made the binary unusable for routine local-dev verification
//! and for the existing CI gate. The current binary instead enforces
//! the same contract from the other end: each fixture is hand-authored
//! against the wire spec, and this binary deserializes every fixture's
//! response body through the typed `Envelope<T>` to guarantee shape
//! conformance. A drift in the api-types crate that breaks an SDK now
//! breaks this gate at the same time, so SDKs and fixtures cannot
//! diverge silently.
//!
//! Adding a new fixture: drop a `<name>/{description.md, request.json,
//! response.json}` triplet into `contracts/fixtures/`, then add the
//! `<name>` to `scenario_kind` mapping it to its response shape. The
//! binary fails on any unknown directory, so the manifest cannot drift
//! out of sync.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sealstack_api_types::{
    connectors::{ListConnectorsResponse, RegisterConnectorResponse, SyncConnectorResponse},
    envelope::{Envelope, ErrorDetail},
    health::HealthStatus,
    query::QueryResponse,
    receipts::ReceiptWire,
    schemas::{ApplyDdlResponse, ListSchemasResponse, RegisterSchemaResponse, SchemaMetaWire},
};
use serde_json::Value;

#[derive(Copy, Clone)]
enum ResponseKind {
    Query,
    RegisterSchema,
    ApplyDdl,
    RegisterConnector,
    SyncConnector,
    ListSchemas,
    ListConnectors,
    SchemaMeta,
    Receipt,
    Health,
    Error,
}

fn scenario_kind(name: &str) -> Option<ResponseKind> {
    match name {
        // Happy paths
        "query-success" => Some(ResponseKind::Query),
        "register-schema-success" => Some(ResponseKind::RegisterSchema),
        "apply-ddl-success" => Some(ResponseKind::ApplyDdl),
        "register-connector-success" => Some(ResponseKind::RegisterConnector),
        "sync-connector-success" => Some(ResponseKind::SyncConnector),
        "list-schemas-success" => Some(ResponseKind::ListSchemas),
        "list-connectors-success" => Some(ResponseKind::ListConnectors),
        "get-schema-success" => Some(ResponseKind::SchemaMeta),
        "get-receipt-success" => Some(ResponseKind::Receipt),
        "healthz-success" => Some(ResponseKind::Health),
        "readyz-success" => Some(ResponseKind::Health),
        // Error paths
        "query-policy-denied" => Some(ResponseKind::Error),
        "query-unauthorized" => Some(ResponseKind::Error),
        "apply-ddl-validation-error" => Some(ResponseKind::Error),
        "register-schema-invalid-argument" => Some(ResponseKind::Error),
        "get-receipt-not-found" => Some(ResponseKind::Error),
        "get-schema-not-found" => Some(ResponseKind::Error),
        _ => None,
    }
}

fn main() -> Result<()> {
    let root = repo_fixtures_dir()?;
    let mut errors: Vec<String> = Vec::new();
    let mut count = 0_u32;

    let mut entries: Vec<_> = fs::read_dir(&root)
        .with_context(|| format!("reading {}", root.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        match validate_one(entry.path().as_path(), &name) {
            Ok(()) => count += 1,
            Err(e) => errors.push(format!("{name}: {e:#}")),
        }
    }

    if !errors.is_empty() {
        eprintln!("emit-fixtures: {} fixture(s) failed validation:", errors.len());
        for e in &errors {
            eprintln!("  - {e}");
        }
        bail!("validation failed");
    }
    println!(
        "emit-fixtures: validated {count} fixture(s) under {}",
        root.display()
    );
    Ok(())
}

fn validate_one(dir: &Path, name: &str) -> Result<()> {
    let kind = scenario_kind(name).with_context(|| {
        format!("unknown fixture `{name}`; add it to scenario_kind() or rename the directory")
    })?;
    let req_path = dir.join("request.json");
    let res_path = dir.join("response.json");
    let desc_path = dir.join("description.md");
    if !req_path.exists() {
        bail!("missing request.json");
    }
    if !res_path.exists() {
        bail!("missing response.json");
    }
    if !desc_path.exists() {
        bail!("missing description.md");
    }

    let req: Value = serde_json::from_str(&fs::read_to_string(&req_path)?)
        .context("parsing request.json")?;
    for k in ["method", "path", "headers", "body"] {
        if req.get(k).is_none() {
            bail!("request.json missing `{k}`");
        }
    }

    let res: Value = serde_json::from_str(&fs::read_to_string(&res_path)?)
        .context("parsing response.json")?;
    for k in ["status", "headers", "body"] {
        if res.get(k).is_none() {
            bail!("response.json missing `{k}`");
        }
    }
    let body = res.get("body").expect("checked above").clone();

    match kind {
        ResponseKind::Query => round_trip_success::<QueryResponse>(body),
        ResponseKind::RegisterSchema => round_trip_success::<RegisterSchemaResponse>(body),
        ResponseKind::ApplyDdl => round_trip_success::<ApplyDdlResponse>(body),
        ResponseKind::RegisterConnector => round_trip_success::<RegisterConnectorResponse>(body),
        ResponseKind::SyncConnector => round_trip_success::<SyncConnectorResponse>(body),
        ResponseKind::ListSchemas => round_trip_success::<ListSchemasResponse>(body),
        ResponseKind::ListConnectors => round_trip_success::<ListConnectorsResponse>(body),
        ResponseKind::SchemaMeta => round_trip_success::<SchemaMetaWire>(body),
        ResponseKind::Receipt => round_trip_success::<ReceiptWire>(body),
        ResponseKind::Health => round_trip_success::<HealthStatus>(body),
        ResponseKind::Error => round_trip_error(body),
    }
}

fn round_trip_success<T: serde::de::DeserializeOwned>(body: Value) -> Result<()> {
    let env: Envelope<T> =
        serde_json::from_value(body).context("body does not match Envelope<T>")?;
    match (env.data.is_some(), env.error.is_some()) {
        (true, false) => Ok(()),
        (false, true) => {
            bail!("expected success envelope (data set, error null), got error envelope")
        }
        (true, true) => bail!("envelope has both data and error set"),
        (false, false) => bail!("envelope has neither data nor error set"),
    }
}

fn round_trip_error(body: Value) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct ErrorEnv {
        data: Option<Value>,
        error: Option<ErrorDetail>,
    }
    let env: ErrorEnv =
        serde_json::from_value(body).context("body is not a valid error envelope")?;
    match (env.data.is_some(), env.error.is_some()) {
        (false, true) => Ok(()),
        (true, _) => bail!("error fixture has data set"),
        (false, false) => bail!("error fixture has no error detail"),
    }
}

fn repo_fixtures_dir() -> Result<PathBuf> {
    // crates/sealstack-api-types -> repo root -> contracts/fixtures
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent()
        .and_then(Path::parent)
        .context("could not resolve repo root from manifest dir")?
        .to_path_buf();
    Ok(repo_root.join("contracts").join("fixtures"))
}
