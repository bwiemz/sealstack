//! Gateway-side policy-engine selection.
//!
//! A thin wrapper that turns "here is an optional directory of compiled
//! `<ns>.<schema>.wasm` bundles" into a [`PolicyEngine`] trait object. The
//! binary reads the env vars and calls [`policy_from_dir`]; integration tests
//! call the same function with a tempdir so they exercise the identical code
//! path without mutating process-global env state.

use std::sync::Arc;

use sealstack_engine::policy::{PolicyEngine, WasmPolicy, default_dev_policy};

/// Which policy-engine backend to construct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PolicyBackend {
    /// CSL-compiled WASM bundles (the default, ships with the workspace).
    #[default]
    Wasm,
    /// Cedar ABAC bundles. Bundle filenames: `<namespace>.<schema>.cedar`.
    Cedar,
}

impl PolicyBackend {
    /// Parse a backend selector from the `SEALSTACK_POLICY_BACKEND` env var
    /// value. Unknown values fall back to [`PolicyBackend::Wasm`] with a
    /// warning so a typo doesn't fail-closed the whole gateway.
    #[must_use]
    pub fn from_env_value(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "" | "wasm" => Self::Wasm,
            "cedar" => Self::Cedar,
            other => {
                tracing::warn!(
                    requested = %other,
                    "unknown SEALSTACK_POLICY_BACKEND; defaulting to wasm",
                );
                Self::Wasm
            }
        }
    }
}

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
    policy_from_dir_with_backend(dir, deny_missing, PolicyBackend::Wasm)
}

/// Same as [`policy_from_dir`] but lets the caller pick between the WASM and
/// Cedar backends. Other behavior is identical — empty dir → allow-all,
/// load failure → warn + fall back.
#[must_use]
pub fn policy_from_dir_with_backend(
    dir: Option<&str>,
    deny_missing: bool,
    backend: PolicyBackend,
) -> Arc<dyn PolicyEngine> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else {
        return default_dev_policy();
    };
    match backend {
        PolicyBackend::Wasm => {
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
        PolicyBackend::Cedar => {
            match sealstack_policy_cedar::build(Some(std::path::Path::new(dir)), deny_missing) {
                Ok(p) => {
                    tracing::info!(%dir, deny_missing, "cedar policy engine initialized");
                    p
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        %dir,
                        "cedar policy init failed; falling back to AllowAllPolicy",
                    );
                    default_dev_policy()
                }
            }
        }
    }
}
