//! Ingestion scheduler: drives connector `list` / `subscribe` calls.

use sealstack_common::SealStackResult;

/// Run a single sync pass for a connector. Stub.
///
/// # Errors
///
/// Returns an error if the connector call fails or the downstream pipeline
/// rejects a resource.
pub async fn sync_once(_connector_name: &str) -> SealStackResult<()> {
    Ok(())
}
