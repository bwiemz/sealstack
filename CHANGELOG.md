# Changelog

All notable changes to this project will be documented in this file. Format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions
follow [SemVer](https://semver.org/).

Maintained automatically by `release-please`.

## [Unreleased]

### Added
- `sealstack compile` now emits Python `TypedDict` record types to
  `out/py/generated.py` alongside the existing Rust and TypeScript outputs.
  Zero external deps (Python 3.11+ stdlib only); `Literal` aliases for
  enums; `<SCHEMA>_META: Final` dicts for reflection; functional
  TypedDict fallback for schemas whose fields collide with Python
  keywords. Deliberately out of scope: Pydantic flag (future slice),
  runtime validation, rich types (`datetime`, `uuid.UUID`), sdks/python
  migration.
- `sealstack compile` now emits TypeScript record types to `out/ts/generated.ts`
  alongside the existing `out/rust/generated.rs`. Plain TypeScript interfaces,
  string-literal-union enums, per-schema `<Name>Meta` constants for reflection.
  No runtime deps; no validation-library wrappers. Deliberately out of scope:
  Python codegen (separate slice), SvelteKit console migration, fetch-client
  generation.
- `sealstack compile` now emits typed Rust structs to `out/rust/generated.rs`
  and WASM policy bundles to `out/policy/<namespace>.<schema>.wasm`.
  Bundles drop straight into a directory configured via `SEALSTACK_POLICY_DIR`
  for the gateway's `WasmPolicy` to load — no hand-authored WAT required.
- New `CompileTargets::WASM_POLICY` flag and `CompileOutput::policy_bundles`
  field in the `sealstack-csl` public API.
- New `sealstack-policy-runtime` crate (no-std, wasm32 target) providing the
  interpreter for CSL policy predicates compiled to a compact IR.
- Initial repository scaffold.
