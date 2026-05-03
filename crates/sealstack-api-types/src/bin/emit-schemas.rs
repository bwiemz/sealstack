//! Emit JSON Schema for every wire type into `schemas/`.
//!
//! CI runs this and verifies the output matches the checked-in copy.
//! See `crates/sealstack-api-types/README.md`.

use std::fs;
use std::path::{Path, PathBuf};

use schemars::{schema_for, JsonSchema};
use sealstack_api_types::{
    connectors::{
        ConnectorBindingWire, ListConnectorsResponse, RegisterConnectorRequest,
        RegisterConnectorResponse, SyncConnectorResponse,
    },
    envelope::{Envelope, ErrorCode, ErrorDetail},
    health::{HealthStatus, HealthStatusKind},
    query::{QueryHit, QueryRequest, QueryResponse},
    receipts::{ReceiptSource, ReceiptWire},
    schemas::{
        ApplyDdlRequest, ApplyDdlResponse, ListSchemasResponse, RegisterSchemaRequest,
        RegisterSchemaResponse, SchemaMetaWire,
    },
};

const VERSION: &str = "v0.3.0";

fn main() -> anyhow::Result<()> {
    let dir = manifest_dir().join("schemas");
    fs::create_dir_all(&dir)?;

    write::<ErrorDetail>(&dir, "ErrorDetail")?;
    write::<ErrorCode>(&dir, "ErrorCode")?;
    write::<HealthStatus>(&dir, "HealthStatus")?;
    write::<HealthStatusKind>(&dir, "HealthStatusKind")?;
    write::<QueryRequest>(&dir, "QueryRequest")?;
    write::<QueryResponse>(&dir, "QueryResponse")?;
    write::<QueryHit>(&dir, "QueryHit")?;
    write::<RegisterSchemaRequest>(&dir, "RegisterSchemaRequest")?;
    write::<RegisterSchemaResponse>(&dir, "RegisterSchemaResponse")?;
    write::<ApplyDdlRequest>(&dir, "ApplyDdlRequest")?;
    write::<ApplyDdlResponse>(&dir, "ApplyDdlResponse")?;
    write::<SchemaMetaWire>(&dir, "SchemaMetaWire")?;
    write::<ListSchemasResponse>(&dir, "ListSchemasResponse")?;
    write::<RegisterConnectorRequest>(&dir, "RegisterConnectorRequest")?;
    write::<RegisterConnectorResponse>(&dir, "RegisterConnectorResponse")?;
    write::<ConnectorBindingWire>(&dir, "ConnectorBindingWire")?;
    write::<ListConnectorsResponse>(&dir, "ListConnectorsResponse")?;
    write::<SyncConnectorResponse>(&dir, "SyncConnectorResponse")?;
    write::<ReceiptWire>(&dir, "ReceiptWire")?;
    write::<ReceiptSource>(&dir, "ReceiptSource")?;

    // Envelope is generic; emit one instantiation per response type used by
    // the SDKs. Schema $id includes both the envelope and the inner type.
    write::<Envelope<QueryResponse>>(&dir, "Envelope_QueryResponse")?;
    write::<Envelope<RegisterSchemaResponse>>(&dir, "Envelope_RegisterSchemaResponse")?;

    let count = fs::read_dir(&dir)?
        .filter(|e| e.as_ref().is_ok_and(|d| d.path().extension().and_then(|s| s.to_str()) == Some("json")))
        .count();
    println!("emitted {count} schemas");
    Ok(())
}

fn write<T: JsonSchema>(dir: &Path, name: &str) -> anyhow::Result<()> {
    let mut schema = schema_for!(T);
    // Stamp the $id with our SemVer so consumers can introspect compat.
    schema.schema.metadata().id =
        Some(format!("https://contracts.sealstack.dev/api-types/{VERSION}/{name}.json"));
    let json = serde_json::to_string_pretty(&schema)?;
    let path = dir.join(format!("{name}.json"));
    fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
