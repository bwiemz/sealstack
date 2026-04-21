//! WASM policy bundle codegen. One bundle per schema, regardless of whether
//! a `policy { ... }` block is present — empty policies emit a bundle whose
//! runtime reads "deny all" so fail-closed deployments behave uniformly.

use crate::error::CslResult;
use crate::types::TypedFile;

/// A compiled WASM policy bundle, ready to write as `<namespace>.<schema>.wasm`.
#[derive(Clone, Debug)]
pub struct PolicyBundle {
    /// CSL namespace (empty string becomes "default" in the filename).
    pub namespace: String,
    /// CSL schema name.
    pub schema: String,
    /// Raw WASM bytes.
    pub wasm: Vec<u8>,
}

/// Emit one bundle per schema.
///
/// Populated over subsequent plan tasks; for now returns an empty Vec so the
/// CompileOutput field has a real shape to bind against.
pub fn emit_policy_bundles(_typed: &TypedFile) -> CslResult<Vec<PolicyBundle>> {
    Ok(Vec::new())
}
