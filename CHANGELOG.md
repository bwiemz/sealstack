# Changelog

All notable changes to this project will be documented in this file. Format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions
follow [SemVer](https://semver.org/).

Maintained automatically by `release-please`.

## [Unreleased]

### Added
- `sealstack-connector-sdk` hardened from a flat 290-line `lib.rs` into focused
  modules: `auth` (`Credential` trait + `StaticToken` impl with redacted Debug
  via `secrecy::SecretString`), `http` (`HttpClient` with reactive retry
  middleware — 401 invalidate-once, 408/429/5xx exponential backoff with
  full jitter, `Retry-After` honoring, hard-cap-protected streaming
  body-size enforcement, baked-in User-Agent with optional connector
  suffix), `retry` (policy + integer-seconds `Retry-After` parser),
  `paginate` (`Paginator` trait with cursor-loop detection, plus three
  reference builders: `BodyCursorPaginator` for cursor-in-body APIs,
  `LinkHeaderPaginator` for RFC 8288 `Link: rel="next"` headers, and
  `OffsetPaginator` for numeric `start`/`limit` against a `total`).
- New `SealStackError` variants in `sealstack-common`: `RetryExhausted`,
  `BodyTooLarge`, `PaginatorCursorLoop`, `HttpStatus { status, headers,
  body }` (with streaming-capped capture for non-retryable 4xx).
- Existing connectors refactored onto the new SDK: `local-files`
  (verification probe — confirms the `Connector` trait isn't coupled to
  HTTP); `slack` (uses `HttpClient` + `BodyCursorPaginator` for channels
  and messages, `stream.take(cap)` for exact cap enforcement, config-wins
  precedence over `SLACK_BOT_TOKEN`); `github` (uses `HttpClient` with
  `github-connector/<ver>` UA suffix + `LinkHeaderPaginator` for repos and
  issues, plus a connector-local `retry_shim` that discriminates GitHub's
  three 403 patterns — primary rate limit, secondary rate limit, and real
  permission denial — for the non-paginated request path).
- Deliberately out of scope: OAuth 2.0 authorization-code flow (lands with
  the Google Drive connector slice), proactive token-bucket rate limiting,
  streaming upload support, typed `PermissionPredicate` shape.
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
