//! Policy predicate evaluation.
//!
//! CSL `policy { read: <expr> }` blocks compile to WASM predicates
//! (`sealstack_csl::codegen::policy`, not yet emitted). At runtime, this module loads
//! those `.wasm` bundles, instantiates them with `wasmtime`, and evaluates them
//! against `(caller, self)` pairs for each candidate record.
//!
//! # v0.1 status
//!
//! The WASM backend is not yet implemented. The trait below is the stable
//! interface; [`AllowAllPolicy`] is the default v0.1 implementation and will be
//! swapped out when codegen emits real policy bundles.
//!
//! # Policy invariants
//!
//! 1. **Default deny when configured.** In production deployments, the engine
//!    should never fall back to allow-all. [`AllowAllPolicy`] logs a warning
//!    at construction to surface accidental use.
//!
//! 2. **Actions are a closed set.** `read`, `write`, `list`, `delete`. Any new
//!    action name is rejected at CSL compile time, so the runtime does not
//!    validate action strings — it assumes the compiler did.
//!
//! 3. **Evaluation is side-effect-free.** Policies may not mutate state, call
//!    external services, or allocate beyond the WASM linear memory.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::{Caller, EngineError};

/// What the policy is being asked to decide.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    /// Allowed to retrieve / view the record.
    Read,
    /// Allowed to modify the record.
    Write,
    /// Allowed to know the record exists (list without read).
    List,
    /// Allowed to delete the record.
    Delete,
}

/// Verdict returned from the policy engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    /// Caller may perform the action on the record.
    Allow,
    /// Caller may not; reason is human-readable.
    Deny {
        /// Reason (do not leak record contents through this string).
        reason: String,
    },
}

impl PolicyVerdict {
    /// True if the verdict is [`Allow`](PolicyVerdict::Allow).
    #[must_use]
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// One policy-evaluation request.
#[derive(Debug, Clone)]
pub struct PolicyInput<'a> {
    /// CSL namespace of the schema the record belongs to.
    pub namespace: &'a str,
    /// CSL schema name.
    pub schema: &'a str,
    /// Action being tested.
    pub action: PolicyAction,
    /// Authenticated caller.
    pub caller: &'a Caller,
    /// The record itself, as a JSON object.
    pub record: &'a Value,
}

/// The policy engine trait.
///
/// An implementation must be cheap to share via `Arc` and cheap to invoke in
/// parallel from many async tasks. The typical implementation keeps a pool of
/// `wasmtime::Instance`s keyed by `(namespace, schema, version)`.
#[async_trait]
pub trait PolicyEngine: Send + Sync + 'static {
    /// Evaluate a policy predicate.
    ///
    /// Implementations return `Ok(Deny { .. })` for a valid but-denying predicate
    /// and reserve `Err(...)` for infrastructural failures (bundle missing,
    /// WASM trap, etc.).
    async fn evaluate(&self, input: PolicyInput<'_>) -> Result<PolicyVerdict, EngineError>;

    /// Filter a batch of records down to those where the caller has `action`.
    ///
    /// Default impl loops. Implementations that can batch evaluate (e.g. a
    /// WASM module with a `evaluate_batch` export) should override.
    async fn filter<'a>(
        &'a self,
        namespace: &'a str,
        schema: &'a str,
        action: PolicyAction,
        caller: &'a Caller,
        records: &'a [Value],
    ) -> Result<Vec<bool>, EngineError> {
        let mut out = Vec::with_capacity(records.len());
        for record in records {
            let v = self
                .evaluate(PolicyInput {
                    namespace,
                    schema,
                    action,
                    caller,
                    record,
                })
                .await?;
            out.push(v.is_allow());
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// AllowAllPolicy — v0.1 default.
// ---------------------------------------------------------------------------

/// Policy engine that approves every request.
///
/// **Do not use in production.** This exists so the engine can function before
/// the WASM policy codegen lands in `sealstack-csl`. The [`Engine::new_dev`] constructor
/// wires it automatically, with a warning log.
pub struct AllowAllPolicy;

#[async_trait]
impl PolicyEngine for AllowAllPolicy {
    async fn evaluate(&self, _input: PolicyInput<'_>) -> Result<PolicyVerdict, EngineError> {
        Ok(PolicyVerdict::Allow)
    }

    async fn filter<'a>(
        &'a self,
        _namespace: &'a str,
        _schema: &'a str,
        _action: PolicyAction,
        _caller: &'a Caller,
        records: &'a [Value],
    ) -> Result<Vec<bool>, EngineError> {
        Ok(vec![true; records.len()])
    }
}

/// Construct the default v0.1 policy engine (allow-all) with a warning log.
#[must_use]
pub fn default_dev_policy() -> Arc<dyn PolicyEngine> {
    tracing::warn!(
        "using AllowAllPolicy — every record is visible to every caller. \
         This is appropriate only for dev and CI; never for production."
    );
    Arc::new(AllowAllPolicy)
}

// ---------------------------------------------------------------------------
// WasmPolicy — wasmtime-backed implementation.
// ---------------------------------------------------------------------------

/// Policy engine that evaluates CSL `policy { ... }` blocks compiled to WASM.
///
/// # Bundle layout
///
/// On construction, [`WasmPolicy::load_from_dir`] walks the given directory
/// and registers every file matching `<namespace>.<schema>.wasm`. Files that
/// don't parse are logged and skipped — one malformed bundle must not block
/// the rest of the engine from booting.
///
/// # WASM ABI v0.1
///
/// Every bundle must export:
///
/// * `memory` — the default linear memory, minimum 1 page.
/// * `sealstack_alloc(n_bytes: i32) -> i32` — bump/linear allocator returning a
///   linear-memory offset the host may write `n_bytes` into.
/// * `sealstack_evaluate(input_ptr: i32, input_len: i32) -> i32` — takes a UTF-8
///   JSON input ([`PolicyInputWire`]) and returns:
///     * `1` — allow
///     * `0` — deny (host synthesizes a generic reason)
///     * negative — host reports as [`EngineError::Backend`]
///
/// The ABI is intentionally narrow — extending it to carry structured deny
/// reasons is a v0.2 change once we've shipped at least one compiled policy.
///
/// # Behaviour when no bundle is registered
///
/// Schemas without an entry in the directory get a default [`PolicyVerdict`].
/// [`Self::load_from_dir`] defaults it to [`PolicyVerdict::Allow`] so schemas
/// with no `policy {}` block keep working. Pass [`WasmPolicy::deny_missing`]
/// to flip to fail-closed if you never want a missing bundle to silently
/// allow access.
#[cfg(feature = "wasm-policy")]
pub struct WasmPolicy {
    engine: wasmtime::Engine,
    modules: dashmap::DashMap<(String, String), wasmtime::Module>,
    default_verdict: PolicyVerdict,
}

#[cfg(feature = "wasm-policy")]
impl WasmPolicy {
    /// Load every `<namespace>.<schema>.wasm` bundle from `dir`.
    ///
    /// Missing-bundle default is [`PolicyVerdict::Allow`]. Use
    /// [`Self::load_from_dir_deny_missing`] when the deployment never runs
    /// schemas without a compiled policy and missing bundles should fail
    /// closed.
    pub fn load_from_dir(dir: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        Self::load_impl(dir.as_ref(), PolicyVerdict::Allow)
    }

    /// Fail-closed variant: missing bundles deny.
    pub fn load_from_dir_deny_missing(
        dir: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Self> {
        Self::load_impl(
            dir.as_ref(),
            PolicyVerdict::Deny {
                reason: "no policy bundle registered for this schema".into(),
            },
        )
    }

    fn load_impl(dir: &std::path::Path, default_verdict: PolicyVerdict) -> anyhow::Result<Self> {
        let engine = wasmtime::Engine::default();
        let modules = dashmap::DashMap::<(String, String), wasmtime::Module>::new();

        if !dir.exists() {
            tracing::warn!(
                path = %dir.display(),
                "WASM policy dir does not exist; registry is empty",
            );
            return Ok(Self {
                engine,
                modules,
                default_verdict,
            });
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            // Expected: `<namespace>.<schema>`. The last dot splits the two.
            let Some((namespace, schema)) = stem.rsplit_once('.') else {
                tracing::warn!(file = %path.display(), "skipping unparseable wasm filename");
                continue;
            };
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, file = %path.display(), "wasm read failed");
                    continue;
                }
            };
            match wasmtime::Module::new(&engine, &bytes) {
                Ok(m) => {
                    tracing::info!(namespace, schema, "registered policy module");
                    modules.insert((namespace.to_owned(), schema.to_owned()), m);
                }
                Err(e) => {
                    tracing::error!(error = %e, file = %path.display(), "wasm compile failed");
                }
            }
        }

        Ok(Self {
            engine,
            modules,
            default_verdict,
        })
    }

}

/// Wire shape for policy input passed to each WASM bundle. Kept separate from
/// [`PolicyInput`] so the bundle does not see internal borrow lifetimes.
#[cfg(feature = "wasm-policy")]
#[derive(Serialize)]
struct PolicyInputWire<'a> {
    namespace: &'a str,
    schema: &'a str,
    action: PolicyAction,
    caller: &'a Caller,
    record: &'a Value,
}

#[cfg(feature = "wasm-policy")]
#[async_trait]
impl PolicyEngine for WasmPolicy {
    async fn evaluate(&self, input: PolicyInput<'_>) -> Result<PolicyVerdict, EngineError> {
        let Some(module_ref) = self
            .modules
            .get(&(input.namespace.to_owned(), input.schema.to_owned()))
        else {
            return Ok(self.default_verdict.clone());
        };
        let module = module_ref.clone();
        drop(module_ref);

        let wire = PolicyInputWire {
            namespace: input.namespace,
            schema: input.schema,
            action: input.action,
            caller: input.caller,
            record: input.record,
        };
        let payload = serde_json::to_vec(&wire)
            .map_err(|e| EngineError::Backend(format!("policy serialize: {e}")))?;

        // WASM evaluation is CPU-bound and synchronous; run it on the blocking
        // pool so it does not stall the async runtime on a slow predicate.
        let engine_clone = self.engine.clone();
        let code = tokio::task::spawn_blocking(move || {
            // Rebuild the store-bound call on the blocking thread so wasmtime
            // state never crosses the boundary back into the async task.
            let policy = WasmPolicyHandle {
                engine: engine_clone,
                module,
            };
            policy.call(&payload)
        })
        .await
        .map_err(|e| EngineError::Backend(format!("wasm task join: {e}")))??;

        match code {
            1 => Ok(PolicyVerdict::Allow),
            0 => Ok(PolicyVerdict::Deny {
                reason: format!("policy denied for {}.{}", input.namespace, input.schema),
            }),
            other => Err(EngineError::Backend(format!(
                "wasm policy returned unexpected code {other} for {}.{}",
                input.namespace, input.schema
            ))),
        }
    }
}

/// Minimal binding used on the blocking thread so `Engine` and `Module` can
/// cross the `spawn_blocking` boundary without dragging the full `WasmPolicy`
/// (and its `DashMap`) into the closure.
#[cfg(feature = "wasm-policy")]
struct WasmPolicyHandle {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
}

#[cfg(feature = "wasm-policy")]
impl WasmPolicyHandle {
    fn call(&self, payload: &[u8]) -> Result<i32, EngineError> {
        let mut store = wasmtime::Store::new(&self.engine, ());
        let instance = wasmtime::Instance::new(&mut store, &self.module, &[])
            .map_err(|e| EngineError::Backend(format!("wasm instantiate: {e}")))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| EngineError::Backend("wasm module missing `memory` export".into()))?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "sealstack_alloc")
            .map_err(|e| EngineError::Backend(format!("wasm alloc: {e}")))?;
        let evaluate = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "sealstack_evaluate")
            .map_err(|e| EngineError::Backend(format!("wasm evaluate: {e}")))?;

        let len_i32 = i32::try_from(payload.len())
            .map_err(|_| EngineError::Backend("policy input too large".into()))?;
        let ptr = alloc
            .call(&mut store, len_i32)
            .map_err(|e| EngineError::Backend(format!("wasm alloc call: {e}")))?;
        memory
            .write(&mut store, ptr as usize, payload)
            .map_err(|e| EngineError::Backend(format!("wasm memory write: {e}")))?;
        evaluate
            .call(&mut store, (ptr, len_i32))
            .map_err(|e| EngineError::Backend(format!("wasm evaluate call: {e}")))
    }
}

#[cfg(all(test, feature = "wasm-policy"))]
mod wasm_tests {
    use super::*;

    /// WAT module that always evaluates to allow (returns 1).
    const ALWAYS_ALLOW: &str = r#"
        (module
          (memory (export "memory") 1)
          (global $bump (mut i32) (i32.const 1024))
          (func (export "sealstack_alloc") (param $n i32) (result i32)
            (local $p i32)
            global.get $bump
            local.tee $p
            local.get $n
            i32.add
            global.set $bump
            local.get $p)
          (func (export "sealstack_evaluate") (param $in_ptr i32) (param $in_len i32) (result i32)
            i32.const 1))
    "#;

    /// WAT module that always evaluates to deny (returns 0).
    const ALWAYS_DENY: &str = r#"
        (module
          (memory (export "memory") 1)
          (global $bump (mut i32) (i32.const 1024))
          (func (export "sealstack_alloc") (param $n i32) (result i32)
            (local $p i32)
            global.get $bump
            local.tee $p
            local.get $n
            i32.add
            global.set $bump
            local.get $p)
          (func (export "sealstack_evaluate") (param $in_ptr i32) (param $in_len i32) (result i32)
            i32.const 0))
    "#;

    fn write_module(dir: &std::path::Path, namespace: &str, schema: &str, wat: &str) {
        let engine = wasmtime::Engine::default();
        let bytes = wat::parse_str(wat).expect("wat parses");
        // Round-trip through wasmtime just to validate before writing.
        wasmtime::Module::new(&engine, &bytes).expect("module compiles");
        let path = dir.join(format!("{namespace}.{schema}.wasm"));
        std::fs::write(&path, &bytes).expect("write wasm");
    }

    #[tokio::test]
    async fn wasm_policy_evaluates_allow_module() {
        let tmp = tempfile::tempdir().unwrap();
        write_module(tmp.path(), "acme", "Doc", ALWAYS_ALLOW);

        let p = WasmPolicy::load_from_dir(tmp.path()).expect("load policy dir");
        let caller = Caller::test("u1");
        let record = serde_json::json!({ "id": "r1" });
        let v = p
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Doc",
                action: PolicyAction::Read,
                caller: &caller,
                record: &record,
            })
            .await
            .unwrap();
        assert!(v.is_allow());
    }

    #[tokio::test]
    async fn wasm_policy_evaluates_deny_module() {
        let tmp = tempfile::tempdir().unwrap();
        write_module(tmp.path(), "acme", "Secret", ALWAYS_DENY);

        let p = WasmPolicy::load_from_dir(tmp.path()).expect("load policy dir");
        let caller = Caller::test("u1");
        let record = serde_json::json!({ "id": "r1" });
        let v = p
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Secret",
                action: PolicyAction::Read,
                caller: &caller,
                record: &record,
            })
            .await
            .unwrap();
        assert!(matches!(v, PolicyVerdict::Deny { .. }));
    }

    #[tokio::test]
    async fn missing_bundle_uses_default_allow() {
        let tmp = tempfile::tempdir().unwrap();
        let p = WasmPolicy::load_from_dir(tmp.path()).expect("load empty dir");
        let caller = Caller::test("u1");
        let record = serde_json::json!({});
        let v = p
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "NoPolicy",
                action: PolicyAction::Read,
                caller: &caller,
                record: &record,
            })
            .await
            .unwrap();
        assert!(v.is_allow());
    }

    #[tokio::test]
    async fn missing_bundle_deny_variant_denies() {
        let tmp = tempfile::tempdir().unwrap();
        let p = WasmPolicy::load_from_dir_deny_missing(tmp.path()).expect("load empty dir");
        let caller = Caller::test("u1");
        let record = serde_json::json!({});
        let v = p
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "NoPolicy",
                action: PolicyAction::Read,
                caller: &caller,
                record: &record,
            })
            .await
            .unwrap();
        assert!(matches!(v, PolicyVerdict::Deny { .. }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allow_all_allows() {
        let p = AllowAllPolicy;
        let record = serde_json::json!({ "id": "x" });
        let caller = Caller::test("u1");
        let v = p
            .evaluate(PolicyInput {
                namespace: "n",
                schema: "S",
                action: PolicyAction::Read,
                caller: &caller,
                record: &record,
            })
            .await
            .unwrap();
        assert!(v.is_allow());
    }

    #[tokio::test]
    async fn allow_all_filter_keeps_all() {
        let p = AllowAllPolicy;
        let caller = Caller::test("u1");
        let records = vec![serde_json::json!({ "a": 1 }), serde_json::json!({ "a": 2 })];
        let mask = p
            .filter("n", "S", PolicyAction::Read, &caller, &records)
            .await
            .unwrap();
        assert_eq!(mask, vec![true, true]);
    }
}
