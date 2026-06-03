# Changelog

All notable changes to this project will be documented in this file. Format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions
follow [SemVer](https://semver.org/).

Maintained automatically by `release-please`.

## [Unreleased]

## [0.4.0] - 2026-06-03

### Added

- **Cedar ABAC policy adapter** (`crates/sealstack-policy-cedar`) — new
  `PolicyEngine` impl backed by [Cedar](https://www.cedarpolicy.com/) 4.x.
  Selected via `SEALSTACK_POLICY_BACKEND=cedar` (default `wasm`). Bundle
  filename convention is `<namespace>.<schema>.cedar`. The entity model
  is `Sealstack::User::"<caller.id>"` parented to one
  `Sealstack::Group::"<g>"` per group + one `Sealstack::Role::"<r>"`
  per role; resources are `Sealstack::Resource::"<record.id>"` with
  every record field surfaced as a Cedar attribute. Missing-bundle
  behavior matches WASM (default Allow, flip to Deny via
  `SEALSTACK_POLICY_DEFAULT=deny`).
- **Five new connectors** bringing the total to 12 (was 7 in v0.3):
  - **linear**: GraphQL `issues` connection with Relay cursor pagination,
    `linear:team:<key>` predicate.
  - **jira**: Jira Cloud REST `/rest/api/3/search` with JQL filter, ADF
    body flattening, `jira:project:<key>` predicate.
  - **confluence**: Confluence Cloud `/wiki/rest/api/content?type=page`
    with storage-format HTML → text via html2text,
    `confluence:space:<key>` predicate.
  - **s3**: AWS SDK `ListObjectsV2` + `GetObject`, endpoint-URL override
    for S3-compatible stores (MinIO / R2 / Ceph), glob include filter,
    `s3:bucket:<bucket>` predicate.
  - **gmail**: REST `/gmail/v1/users/me/messages` with Gmail-query
    filter, MIME-tree text/plain extraction (HTML fallback),
    `gmail:user:<email>` predicate from `From:` header.
- **Chat-style console route** (`/chat`) — alternative to the playground
  with composer at the bottom, transcript in the middle, and a clickable
  receipt link on every assistant turn. Retrieval-only (no LLM in the
  loop); each turn fires one `/v1/query` against the selected schema.
- **Vector-store filter DSL** (`crates/sealstack-vectorstore/src/filter.rs`)
  — MongoDB-style operator vocabulary: `$eq` (or bare scalar), `$ne`,
  `$in`, `$nin`, `$gt`, `$gte`, `$lt`, `$lte`, `$and`, `$or`, `$not`.
  Flat-equals shape stays backwards-compatible. In-memory + Qdrant
  backends both consume the typed `Filter` enum. `$or` / `$not`
  compositions fall back to in-memory post-filter on Qdrant (logged at
  warn so deployments see the cost).
- **Tiktoken-rs token counting** for the chunker (feature
  `tiktoken-chunker`, opt-in; gateway opts in). Real cl100k_base BPE
  counts replace the 4-chars-per-token heuristic for `Semantic` and
  `Recursive` chunking strategies. Char-approx counter still ships as
  the dependency-free fallback.
- **Production-ready Helm chart** — Deployment with full probes
  (`/livez`, `/readyz`, startup probe), non-root security context,
  read-only rootfs, resource floors, ConfigMap-driven env, externalized
  Secret pattern, optional HPA / PDB / ServiceMonitor / NetworkPolicy /
  Ingress.

### Changed

- Release pipeline cleanups (from the v0.3 post-mortem):
  - `cli-binaries` matrix no longer races on tag creation — release is
    pre-created in a `create-release` job, the matrix uploads only.
  - `release-attestations`: dropped the obsolete `--output-pattern`
    flag; per-crate bom.json files collected into `artifacts/`.
  - `publish-ts-sdk`: dropped `--no-git-checks` from the dry-run
    (pnpm-only flag, rejected by `npm publish`).
  - `gateway-image`: Dockerfile scopes the bin build to
    `-p sealstack-gateway` since workspace `default-members` points at
    the CLI crate. Base image bumped 1.85 → 1.88.
- `Range`/`Filter` conversions in the Qdrant backend route through the
  new typed `crate::filter::Filter` rather than ad-hoc JSON walks.

### Fixed

- `webpki` RUSTSEC-2026-0098 / RUSTSEC-2026-0099 (name-constraint
  advisories) added to `deny.toml` ignore list. Both impact only servers
  validating client certificates with name-constraint extensions;
  SealStack uses rustls for outbound HTTPS only.

## [0.3.0] - 2026-06-03

### Added

- **TypeScript SDK** (`@sealstack/client`) — full client implementation,
  generated types from the api-types JSON Schemas, msw-mocked unit tests,
  parametrized fixture corpus tests, smoke suite that runs against a live
  gateway in the integration CI job.
- **Python SDK** (`sealstack`) — same surface as the TS SDK plus
  async-context-manager protocol, sync facade, respx fixture tests,
  integration smoke suite. Pydantic v2 models generated from the same
  JSON Schemas as the TS types.
- **Wire contracts** (`crates/sealstack-api-types/`, `contracts/`) — single
  source of truth for the gateway REST surface, JSON Schemas emitted via
  schemars, fixture corpus consumed by both SDKs, and an `emit-fixtures`
  validator that round-trips every fixture through the typed Rust
  `Envelope<T>` so the corpus and the wire types cannot drift silently.
- **Postgres scrape connector** (`connectors/postgres/`) — single-table
  scrape with identifier allowlist, per-row byte cap, lazy pool, 8-conn
  cap. Configured DSN, table, id column, body columns, optional title
  and updated-at columns.
- **Web (HTTP) connector** (`connectors/web/`) — fixed URL list with
  comprehensive SSRF defense: scheme allowlist, pre-fetch DNS check
  rejecting loopback / RFC 1918 / link-local / unique-local v6 /
  multicast / broadcast / unspecified / documentation IPs; literal-IP
  short-circuit; 3-redirect cap; html2text extraction for HTML.
- **Notion API connector** (`connectors/notion/`) — internal-integration
  PAT auth, `Notion-Version: 2022-06-28` header pinned, lists pages via
  `/v1/search`, flattens top-level block trees. Token baked into auth
  header once and dropped from struct surface.
- **Token-rotation fix** in both SDKs — `bearer({ token: () => string })`
  now actually rotates per-request via a `HeadersSource` factory.
- **Gateway integration in CI** — `integration` job provisions Postgres
  + Qdrant service containers, runs `end_to_end.rs` ignored tests
  serialized, boots the gateway, runs both SDK smoke suites against it.
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
- `sealstack compile` emits Python `TypedDict` record types to
  `out/py/generated.py` alongside the existing Rust and TypeScript outputs.
  Zero external deps (Python 3.11+ stdlib only); `Literal` aliases for
  enums; `<SCHEMA>_META: Final` dicts for reflection; functional
  TypedDict fallback for schemas whose fields collide with Python
  keywords.
- `sealstack compile` emits TypeScript record types to `out/ts/generated.ts`
  alongside `out/rust/generated.rs`. Plain TypeScript interfaces,
  string-literal-union enums, per-schema `<Name>Meta` constants for reflection.
- `sealstack compile` emits typed Rust structs to `out/rust/generated.rs`
  and WASM policy bundles to `out/policy/<namespace>.<schema>.wasm`.
  Bundles drop straight into a directory configured via `SEALSTACK_POLICY_DIR`
  for the gateway's `WasmPolicy` to load — no hand-authored WAT required.
- New `CompileTargets::WASM_POLICY` flag and `CompileOutput::policy_bundles`
  field in the `sealstack-csl` public API.
- New `sealstack-policy-runtime` crate (no-std, wasm32 target) providing the
  interpreter for CSL policy predicates compiled to a compact IR.
- Initial repository scaffold.

### Fixed

- **CSL SQL emitter ↔ engine ingest column drift** — the SQL emitter was
  skipping `@chunked` fields and never producing the `body`, `created_at`,
  `metadata` columns the engine's INSERT hardcodes. Every per-resource
  ingest silently failed; sync returned 200 with zero chunks written;
  retrieval found nothing. Aligned: convention columns emitted on every
  table; `Ulid` and `Ref<T>` map to `text` so ULID-string binds match.
  Three new unit tests lock the invariant. The
  `admin_only_policy_filters_non_admin_results` e2e test now runs in CI
  without `--skip`.
- **Per-resource sync errors no longer silently swallowed** —
  `SyncOutcome.first_resource_error` is populated so the first ingest
  failure surfaces on the structured outcome instead of disappearing
  into `resources_failed`.

### Changed

- Workspace version bumped to `0.3.0` to match the published SDK versions
  and the v0.3 OSS launch milestone.

