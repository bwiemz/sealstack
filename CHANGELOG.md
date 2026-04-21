# Changelog

All notable changes to this project will be documented in this file. Format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions
follow [SemVer](https://semver.org/).

Maintained automatically by `release-please`.

## [Unreleased]

### Added
- `sealstack compile` now emits typed Rust structs to `out/rust/generated.rs`
  and WASM policy bundles to `out/policy/<namespace>.<schema>.wasm`.
  Bundles drop straight into a directory configured via `SEALSTACK_POLICY_DIR`
  for the gateway's `WasmPolicy` to load — no hand-authored WAT required.
- New `CompileTargets::WASM_POLICY` flag and `CompileOutput::policy_bundles`
  field in the `sealstack-csl` public API.
- New `sealstack-policy-runtime` crate (no-std, wasm32 target) providing the
  interpreter for CSL policy predicates compiled to a compact IR.
- Initial repository scaffold.
