//! Gateway-side policy-engine selection.
//!
//! A thin wrapper that turns "here is an optional directory of compiled
//! `<ns>.<schema>.wasm` bundles" into a [`PolicyEngine`] trait object. The
//! binary reads the env vars and calls [`policy_from_dir`]; integration tests
//! call the same function with a tempdir so they exercise the identical code
//! path without mutating process-global env state.

use std::sync::Arc;

use sealstack_engine::policy::{PolicyEngine, WasmPolicy, default_dev_policy};

/// Select the policy engine for a gateway instance.
///
/// Behavior:
///
/// * `dir == None` or `dir == Some("")` → [`default_dev_policy`] (allow-all).
///   Matches the existing "unset env var" behavior of the binary.
/// * `dir == Some(path)` with `deny_missing == false` → [`WasmPolicy::load_from_dir`].
/// * `dir == Some(path)` with `deny_missing == true` → [`WasmPolicy::load_from_dir_deny_missing`].
///
/// On load failure the function logs an error and falls back to the allow-all
/// policy so a misconfigured deployment still boots. Callers who want
/// fail-closed boot must check the return value themselves (or validate the
/// directory ahead of time).
#[must_use]
pub fn policy_from_dir(dir: Option<&str>, deny_missing: bool) -> Arc<dyn PolicyEngine> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else {
        return default_dev_policy();
    };
    let result = if deny_missing {
        WasmPolicy::load_from_dir_deny_missing(dir)
    } else {
        WasmPolicy::load_from_dir(dir)
    };
    match result {
        Ok(p) => {
            tracing::info!(%dir, deny_missing, "wasm policy engine initialized");
            Arc::new(p)
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                %dir,
                "wasm policy init failed; falling back to AllowAllPolicy",
            );
            default_dev_policy()
        }
    }
}
