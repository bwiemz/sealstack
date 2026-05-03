# SealStack SDK Clients — TypeScript + Python (v0.3)

**Status**: design  •  **Phase**: 1 (Public OSS launch)  •  **Date**: 2026-05-02

## 1. Goal and scope

Ship Phase 1's "SDK GA: TypeScript, Python" deliverable
([SealStack-Plan.md §10](../../../design/SealStack-Plan.md)) as two
language SDKs that cover the gateway's full v0.3 REST surface, share
a single canonical contract, and make the v0.3 Quickstart a
copy-pasteable six-line demo.

**In scope**: both languages in parallel; reads (`query`, `list_*`,
`get_*`, `get_receipt`, health probes); admin (`register_schema`,
`apply_ddl`, `register_connector`, `sync_connector`); two-factory
auth (bearer + unauthenticated); typed error hierarchy; retry policy
(aggressive on reads, none on admin); shared wire fixtures;
Quickstart per language.

**Out of scope, deferred to v0.4+**: MCP-from-language-SDK (use the
official MCP SDKs); `Idempotency-Key` support and admin auto-retry
(half-implemented is worse than not at all); OAuth refresh as a
first-class mode (the `bearer(token=callable)` shape in §7 is the
v0.3 escape hatch); Result-style return types (default stays
exception-based; `try_*` companions may land in v0.4); pagination
cursors (`list_*` returns full lists; `list_*_paginated()` companions
in v0.4); OpenTelemetry propagation; the Rust SDK (deferred to v0.5,
foundation laid by `sealstack-api-types`).

## 2. Architecture: three layers

```text
contracts/                       (canonical, hand-written)
├── sdk-contract.md              ← this document
├── fixtures/                    ← request/response pairs per scenario
├── COMPATIBILITY.md             ← gateway-skew matrix
└── CHANGELOG.md                 ← contract-level changes

crates/sealstack-api-types/      (canonical, Rust)
├── src/lib.rs                   ← #[derive(JsonSchema)] structs
├── schemas/*.json               ← emitted JSON Schemas (versioned)
└── bin/emit-schemas.rs          ← regenerate-and-diff CI

sdks/typescript/  +  sdks/python/  (implement the contract)
```

The contract is canonical, not the TS implementation. Type shapes
come from JSON Schema codegen. Endpoint contracts (URL × status ×
retry × idempotency), the wire envelope, and the error taxonomy are
hand-written here. SDKs implement; they are not the source of truth.
When TS and Python disagree the contract breaks the tie; when the
contract is silent that's a contract bug, not a precedent to copy
from whichever language landed first.

## 3. Wire envelope

Every gateway response uses a discriminated-union envelope:

```json
{ "data": <T> | null, "error": null | { "code": "<string>", "message": "<string>" } }
```

The SDK unwraps the envelope. **All public methods return `T` for the
success type, raise from the typed error hierarchy on failure, and
never expose the envelope.** A CI test per SDK asserts this against a
known fixture: the returned object equals the `data` field, not the
envelope.

**Precursor PR**: gateway `crates/sealstack-gateway/src/auth.rs:212`
returns plain-text 401 with a `WWW-Authenticate` header — the only
endpoint that bypasses the JSON envelope. Wrapped in the standard
envelope before this slice begins; the SDK cannot decode 401s
consistently otherwise.

## 4. Endpoint table

| Method | Path | Namespace | Request | Response (`data`) | Error codes | Retriable | Idempotent |
|---|---|---|---|---|---|---|---|
| GET | `/healthz` | top-level | — | `{ status: "ok" }` | `backend` | yes | yes |
| GET | `/readyz` | top-level | — | `{ status: "ok" \| "starting" }` | `backend` | yes | yes |
| POST | `/v1/query` | top-level | `QueryRequest` | `QueryResponse` | `not_found`, `unknown_schema`, `invalid_argument`, `policy_denied`, `backend` | yes | yes |
| GET | `/v1/schemas` | `schemas` | — | `{ schemas: SchemaMeta[] }` | `backend` | yes | yes |
| POST | `/v1/schemas` | `admin.schemas` | `RegisterSchemaRequest` | `{ qualified: str }` | `invalid_argument`, `backend` | **no** | caller-managed in v0.3 |
| GET | `/v1/schemas/{q}` | `schemas` | — | `SchemaMeta` | `not_found`, `backend` | yes | yes |
| POST | `/v1/schemas/{q}/ddl` | `admin.schemas` | `ApplyDdlRequest` | `{ applied: int }` | `not_found`, `invalid_argument`, `backend` | **no** | naturally idempotent server-side |
| GET | `/v1/connectors` | `connectors` | — | `{ connectors: ConnectorBinding[] }` | `backend` | yes | yes |
| POST | `/v1/connectors` | `admin.connectors` | `RegisterConnectorRequest` | `{ id: str }` | `invalid_argument`, `not_found`, `backend` | **no** | caller-managed in v0.3 |
| POST | `/v1/connectors/{id}/sync` | `admin.connectors` | — | `{ jobId: str }` | `not_found`, `backend` | **no** | sync may produce duplicates if retried |
| GET | `/v1/receipts/{id}` | `receipts` | — | `Receipt` | `not_found`, `backend` | yes | yes |

Wire shapes (`QueryRequest`, `SchemaMeta`, etc.) live in
`crates/sealstack-api-types/` (§6) and are emitted as JSON Schema.
The table above is the URL-and-semantics layer the codegen does not
cover.

## 5. API surface

### 5.1 Class shape and factories

A single `SealStack` class per language. Two factory methods on the
class produce instances; the constructor itself is not part of the
public API.

```python
client = SealStack.bearer(url="https://gateway.example.com", token="ya29...")
client = SealStack.unauthenticated(url="http://localhost:7070",
                                   user="alice", tenant="default", roles=["admin"])
```

TS uses static class methods with the same shape:
`SealStack.bearer({ url, token })` /
`SealStack.unauthenticated({ url, user, tenant, roles })`. Static
methods, not free functions exported from the package — keeps the
autocomplete shape identical to Python's, with both factories visible
when typing `SealStack.`.

### 5.2 Two namespaces

Reads sit flat on the client; admin sits under `client.admin`:

```python
# Reads — hot path
client.query(schema=..., query=..., top_k=...)
client.schemas.list()         ;  client.schemas.get("examples.Doc")
client.connectors.list()      ;  client.receipts.get("01JD...")
client.healthz()              ;  client.readyz()

# Admin — one-shot at deploy / CI time
client.admin.schemas.register(meta=...)
client.admin.schemas.apply_ddl(qualified="examples.Doc", ddl=...)
client.admin.connectors.register(kind="local-files", schema="examples.Doc", config={...})
client.admin.connectors.sync("local-files/examples.Doc")
```

The split is namespace organization, not separate classes; it costs
nothing structurally. The dividend: IDE autocomplete distinguishes
`client.query(...)` (safe, frequent) from
`client.admin.schemas.register(...)` (deliberate, rare); typing
`client.adm` triggers the right friction. Admin operations can carry
idempotency keys / dry-run modes / force flags in v0.4 without
polluting the read namespace. Mirrors Stripe / AWS / Kubernetes
client conventions.

### 5.3 Method-naming convention

Names are language-agnostic in the contract, projected per language:
`<contract_name>` → `snake_case` (Python) / `camelCase` (TS). The
namespace already qualifies, so it's `apply_ddl` not
`apply_schema_ddl`, `register` not `register_schema`. Examples:
`schemas.apply_ddl` → `client.admin.schemas.apply_ddl(...)` /
`client.admin.schemas.applyDdl(...)`.

### 5.4 Async-first

Both SDKs are async-native (TS has no choice; fetch is async).
Python ships an async client built on `httpx.AsyncClient` plus a
thin sync facade for callers who do not want to manage an event loop:

```python
async with SealStack.bearer(url=..., token=...) as client:           # canonical
    result = await client.query(...)

with SealStack.bearer(url=..., token=...).sync() as client:           # sync facade
    result = client.query(...)
```

The sync facade runs each async method to completion via
`asyncio.run`. Public surface is identical; signatures are not
duplicated.

## 6. Type source — `crates/sealstack-api-types/`

Wire types live in a standalone Rust crate, not feature-flagged
inside the gateway. Three reasons:

- The crate compiles independently of axum / sqlx / wasmtime; CI
  emit job is fast and isolated.
- Crate boundary signals "this is a wire-format change" to reviewers.
  PRs touching `sealstack-api-types/src/lib.rs` need wire-aware
  review; gateway-handler PRs do not pay that tax.
- The future Rust SDK (v0.5) depends on this crate directly;
  co-locating in the gateway would force the Rust SDK to inherit
  axum/sqlx as transitive deps.

### 6.1 Pipeline

```text
Rust types (#[derive(JsonSchema)])
   │  cargo run --bin emit-schemas -p sealstack-api-types
   ▼
crates/sealstack-api-types/schemas/*.json
   │
   ├── json-schema-to-typescript ──► sdks/typescript/src/generated/*.ts
   └── datamodel-code-generator ──► sdks/python/sealstack/_generated/*.py
```

CI pattern is **regenerate-and-diff**, not regenerate-and-commit
(matches `cargo fmt --check`). PRs that change a Rust type without
regenerating fail with a clear message; same pattern at the SDK
codegen layer.

### 6.2 Pins and gotchas

- **`schemars` 0.8** for v0.3. The 0.8 → 1.0 migration is mechanical
  and queued for after upstream 1.0 stabilizes.
- **Top-level `$id` SemVer on every JSON Schema** (e.g.
  `https://contracts.sealstack.dev/api-types/v0.3.0/QueryRequest.json`).
  Bumped manually for breaking changes; lets future consumers
  introspect compatibility without reverse-engineering CHANGELOG.
- **Day-1 datamodel-code-generator smoke test**: generate pydantic
  models for `SchemaMeta` and `Receipt` early. The tool produces
  awkward output for some discriminated-union shapes; surface this
  before committing to the approach. Mitigations include
  `--use-schema-description`, `--target-python-version 3.11`, and
  occasional hand-massaging of generated `model_config` lines.
- **Three-step PR pattern**: Rust edit → schema regen → SDK type
  regen. Each step is mechanical; CI fails clearly at whichever step
  is skipped.

### 6.3 Codegen division of labor

| Layer | Source | Lives in |
|---|---|---|
| Type shapes (request bodies, response data, enums) | Rust + `JsonSchema` derive → JSON Schema → codegen | `sealstack-api-types/` + emitted schemas |
| Endpoint contracts (URL × status × retry × idempotency) | Hand-written | this doc (§4) |
| Wire envelope (`{data, error}` discriminator) | Hand-written | this doc (§3) + SDKs |
| Error taxonomy (`code` → exception class) | Hand-written | this doc (§8) + SDKs |

JSON Schema describes types, not endpoints. Stating this division
explicitly here preempts the failure mode where someone adds an
endpoint, regenerates schemas, and forgets that the §4 endpoint
table is the canonical thing to keep in sync.

## 7. Auth

### 7.1 Two factories

- **`SealStack.bearer(url, token)`** — production. `token` is a
  string or a zero-arg callable returning a string. The callable form
  is the v0.4 escape hatch for OAuth refresh; absorbing it in the v0.3
  factory means no third factory is needed later.
- **`SealStack.unauthenticated(url, user, tenant, roles)`** — local
  dev. Sends `X-SealStack-User` / `X-SealStack-Tenant` /
  `X-SealStack-Roles` headers. **`tenant` is required**; missing
  tenant must raise `TypeError` (or TS equivalent), not silently
  default. Tenant isolation is a security-relevant boundary.

### 7.2 The name `unauthenticated`

Not `dev`. The factory describes a property (the request is
unauthenticated), not an environment. The runbook-misuse failure
mode — operator on staging reaches for `client.dev(...)` because
"this is dev-ish, right?" — is exactly what the alarming name
prevents. `unauthenticated` reads as alarming the first time, which
is correct.

### 7.3 Runtime warning for non-local URLs

```python
if not _looks_like_local(url):
    warnings.warn(
        f"SealStack.unauthenticated() called against non-local URL {url}. "
        "Production gateways should reject these requests, but you should "
        "use bearer() in any code that runs outside your laptop.",
        stacklevel=2,
    )
```

`_looks_like_local` matches `localhost`, `127.0.0.1`,
`host.docker.internal`, `*.local`. Warning, not error — operators on
a tunnel pointing at `localhost:7070` deserve the warning but should
proceed. TS uses `console.warn` with the same predicate.

### 7.4 The `roles` argument

`SealStack.unauthenticated(user="alice", tenant="default",
roles=["admin"])` is the line that, in the wrong environment, grants
an investigator full admin access without authentication. Flagged
here so future maintainers keep the alarming-name + runtime-warning
combination intact rather than softening it.

## 8. Error taxonomy and class hierarchy

Flat hierarchy: one base class plus one subclass per `code`. No
intermediate `ClientError` / `ServerError` tiers — they are
organizational ceremony that does not earn autocomplete value. If a
real consumer in v0.4 asks for category-level catches, intermediate
classes can be inserted non-breakingly.

| Code | Class | HTTP | Per-class attributes |
|---|---|---|---|
| `not_found` | `NotFoundError` | 404 | `resource: str` |
| `unknown_schema` | `UnknownSchemaError` *(subclass of `NotFoundError`)* | 404 | `schema: str` |
| `unauthorized` | `UnauthorizedError` | 401 | `realm: str \| None` |
| `policy_denied` | `PolicyDeniedError` | 403 | `predicate: str` |
| `invalid_argument` | `InvalidArgumentError` | 400 | `field: str \| None`, `reason: str` |
| `rate_limited` | `RateLimitedError` | 429 | `retry_after: int \| None` *(reserved; gateway emits in v0.4)* |
| `backend` | `BackendError` | 5xx | `request_id: str` |

`UnknownSchemaError` extends `NotFoundError` because the wire
relationship is hierarchical; all other classes extend
`SealStackError` directly.

### 8.1 Disclosure posture

- **`PolicyDeniedError.predicate` is intentional disclosure.** The
  rule name that denied the request is surfaced consistent with
  SealStack's receipt-first posture — the system is auditable, rules
  are not hidden. Some products take the opposite posture; for
  SealStack that's the wrong call.
- **`RateLimitedError.retry_after: int | None`** — optional. Some
  quota exhaustions have no fixed reset window (e.g. "next UTC
  midnight"); optionality avoids defaulting to a sentinel that's
  wrong.
- **`BackendError.request_id` is required.** The gateway must emit
  `X-Request-Id` on every 5xx response; the SDK populates this
  attribute from the header. CI verifies header presence on the
  gateway's error paths. This is the diagnostic field that pays for
  itself the first time a customer files a ticket.

### 8.2 Closed-set `code` enum

The `code` field in the error envelope is an explicit string enum in
JSON Schema with all canonical values. Both SDK codegen pipelines
produce typed dispatch. If the gateway returns an unknown code, the
SDK falls back to `BackendError` with a "unknown error code: <code>"
message — same conservative-default pattern as the connector SDK's
"unknown role → read."

### 8.3 TypeScript: `class extends Error`, not type unions

Each error class extends `SealStackError` extends `Error`.
`instanceof` checks must work at runtime; the discriminator is the
`name` field, set in each constructor. Type unions are forbidden:
they support compile-time narrowing but not the runtime `instanceof`
shape Python's idiom relies on.

### 8.4 v0.4 escape hatch: `try_*` variants

Pinned now to preempt future "should we have done Result from the
start?" debate. v0.4 may add `try_*` companions to every method,
returning a `Result<T, SealStackError>` discriminated union. Default
stays exception-based; `try_*` is opt-in for callers who want
compile-time exhaustiveness. Same pattern as `list_*_paginated`
(§11).

### 8.5 Tests per class

Three required tests per error class: (1) constructor populates
attributes, (2) `instanceof SealStackError` / `isinstance(...,
SealStackError)` is `True`, (3) **wire-shape parsing dispatches to
the right class given a `code` value** — table-driven across all
codes, this is the load-bearing test for the dispatch contract.

## 9. Retry, timeout, cancellation

### 9.1 Read namespace

Auto-retry on network errors (`ECONNREFUSED`, DNS, TLS handshake),
HTTP 429 (honoring `Retry-After`), and **the entire HTTP 5xx class**.
Max 3 attempts (= 2 retries). Exponential backoff: 200 ms, 400 ms,
800 ms, with **full jitter** — actual delay is uniform random in
`[0, base * 1.25]`. Total deadline = per-call timeout; do not retry
past it.

The full-5xx posture matches AWS / Stripe / GCloud SDKs for read
operations. The "500 might mean state corruption" carve-out does not
apply: reads are stateless; a 500 the gateway didn't classify as
permanent is by definition transient.

### 9.2 Admin namespace

**Zero auto-retry in v0.3.** Admin failures raise immediately. CI
runners are happy to retry the whole step; the failure mode is
visible. v0.4 adds idempotency-key support; admin methods gain an
`idempotency_key=` parameter, and `client.admin.with_auto_retry()`
returns a wrapper that auto-generates keys.

### 9.3 Timeouts

Defaults: reads 30 s, admin 60 s. Admin's 60 s is a hard SDK ceiling,
not a target — the gateway aims for sub-30 s DDL applies as a soft
contract. Per-call override: `timeout=` only. Retry policy is
client-level — a caller wanting per-call retry tuning has bigger
problems than the SDK should accommodate.

### 9.4 Cancellation propagation

Python's `KeyboardInterrupt` / `asyncio.CancelledError` and TS's
`AbortSignal` propagate **through** the retry sleep, not just at
boundaries. A caller aborting mid-retry-sleep cancels the sleep
immediately. Implementation: `asyncio.wait_for` over sleep + cancel
source (Python); `Promise.race` over sleep + abort signal (TS).

### 9.5 Knob naming

- `retry_attempts=N` means **N retries (N+1 total attempts)**. `0`
  is "fail immediately, no retries." Documented explicitly.
- Parameter is `retry_initial_backoff_ms`, not
  `retry_initial_backoff` — unit in the name.
- All knobs at client construction:
  `SealStack.bearer(url=..., token=..., timeout=30, retry_attempts=2,
  retry_initial_backoff_ms=200)`.

## 10. Observability

Opt-in via `SealStack.bearer(..., debug=True)` constructor flag *or*
`SEALSTACK_SDK_DEBUG=1` environment variable. Either turns on
wire-level request/response logging.

**What gets logged:**

- Always: method, path, status, latency, request ID
  (`X-Request-Id` from response).
- Headers: yes, **with redaction** (§10.1).
- Request body: **never** (caller-supplied; may contain secrets).
- Response body on success: **never** (may contain retrieved PII).
- Response body on error (4xx/5xx): **yes, in full** (error envelopes
  are diagnostic information by construction).

The asymmetry is deliberate: errors are exactly the case the debug
flag exists to surface, and error envelopes contain only diagnostic
content. "Log nothing ever" makes debug useless; "log everything
always" leaks secrets.

### 10.1 Header redaction list

Logged as `<redacted>` (not omitted, so the developer sees the
header was present). Case-insensitive comparisons.

- `authorization`, `cookie`, `x-api-key`
- `x-sealstack-user`, `x-sealstack-tenant`, `x-sealstack-roles`
- `x-cfg-user`, `x-cfg-tenant`, `x-cfg-roles` *(legacy; CLI emits
  these pre-rebrand)*

Adding a header to the redaction list is a contract change. The list
is a constant in the SDK contract layer and shared across both
languages. v0.3 has no `tracing`-style structured spans, no
OpenTelemetry propagation; v0.4 may add OTel if a real consumer
asks.

## 11. Pagination

`list_*` methods return `list[T]` for v0.3; workspaces are small
(≤10 schemas, ≤30 connectors typical), pre-fetching is cheap. v0.4
adds `list_*_paginated()` companion methods returning an async
iterator — non-breaking; companion-not-replacement pattern mirrors
`try_*` (§8.4). Consumers learn the convention once.

## 12. Testing

Hybrid: shared wire fixtures drive language-level unit tests; small
smoke suite per SDK against a live gateway in the existing CI
`integration` job (PR #45).

### 12.1 Fixture corpus — `contracts/fixtures/`

Each scenario is a directory with three files. `description.md` is
**required**, not optional — a fixture without a description cannot
be reviewed:

```text
contracts/fixtures/query-success/
├── description.md       ← ~3 lines: what scenario, why
├── request.json         ← { method, path, headers, body }
└── response.json        ← { status, headers, body }
```

`request.json`/`response.json` carry the full envelope (method/path
on request; status/headers on response) so the SDK replay covers
every wire detail — `Authorization` header presence, `Retry-After`
on `RateLimited`, etc.

### 12.2 Naming convention

`<endpoint-or-namespace>-<outcome>`:
`query-success`, `query-policy-denied`, `query-rate-limited`,
`register-schema-success`, `register-schema-conflict`,
`apply-ddl-validation-error`, `get-receipt-not-found`.

### 12.3 Coverage target for v0.3

~25 fixtures: 12 happy-path (one per endpoint in §4) + ~13 error
scenarios spanning the taxonomy. **Every error class in §8 is
exercised by at least one fixture.** The full Cartesian product
(12 × 7 = 84) is too much for v0.3; the constraint is "every
dispatch-table entry has at least one test."

### 12.4 Cross-language symmetry

Each SDK's tests include a `corpus_coverage` listing the fixtures it
consumes. CI greps both lists and fails on asymmetry — a fixture not
consumed by both languages is a build failure. This is how the
canonical-contract promise gets teeth: divergence becomes CI
failure, not documentation drift.

### 12.5 Smoke suite per SDK (5 cases each)

Run against the live gateway in the `integration` job: (1)
happy-path read (`query` with non-empty results, verify the receipt
URL resolves), (2) happy-path admin (`register_schema`), (3) error
case (`PolicyDeniedError`), (4) auth path (`UnauthorizedError` from
`bearer()` with bad token), (5) cross-cutting (query → receipt
fetch). Bounded at 10 cases per SDK to keep the integration job
under 15 minutes. Mock libs: `msw` 2.x (TS), `respx` (Python).

### 12.6 Nightly drift-check CI job

The fixture emitter doubles as a smoke test against the live
gateway: nightly cron runs `cargo run --bin emit-fixtures -p
sealstack-api-types` against the latest gateway and diffs the output
against the checked-in corpus. A diff means either the gateway
changed and fixtures need refresh, or a regression slipped in. This
is distinct from the PR-time regenerate-and-diff: the nightly check
catches **historic-corpus staleness** that no PR touched.

## 13. Versioning and compatibility

Both SDKs use independent semver. Both are pre-1.0 in this slice
(`sealstack ^0.3.0` Python, `@sealstack/client ^0.3.0` npm).
`CHANGELOG.md` per package; `contracts/CHANGELOG.md` covers wire-
shape changes affecting both SDKs. Pre-1.0 freedom permits breaking
changes between minors but is treated as a budget — spend it on
real wins, not cleanups that could wait for 1.0.

### 13.1 Compatibility matrix — `contracts/COMPATIBILITY.md`

Single artifact tracking which SDK versions speak which gateway
versions. Becomes load-bearing post-1.0; starts now with one row.

### 13.2 Skew policy

**Each SDK X.Y supports gateway X.Y and gateway X.(Y-1).** Lets
operators choose deploy order (SDK-first or gateway-first) for
rolling updates while keeping the compatibility surface bounded.
Older-than-(Y-1) is explicitly out of scope — compatibility-matrix
complexity goes nonlinear past one minor.

## 14. Package metadata

- **Names**: `sealstack` (PyPI; bare name); `@sealstack/client`
  (npm; scoped). The bare `sealstack` npm name is **reserved** for
  a future meta-package.
- **License**: Apache-2.0; headers in every source file.
- **Python floor**: 3.11+. Rationale: `typing.Self` for error class
  self-references, structural pattern-matching for `try_*` Result
  handling, `tomllib` in stdlib. 3.10 loses typing features; 3.12+
  is too aggressive for enterprise pinning.
- **No lifecycle hooks**: the npm package contains **no
  `postinstall`, `preinstall`, or other lifecycle hooks**. Pure
  metadata + module copy. Enterprise security teams blocklist
  packages with postinstall hooks reflexively; CI verifies the
  absence. This is a contract requirement, not a recommendation.
- **Per-package READMEs** link to `contracts/sdk-contract.md` (this
  file) and embed the Quickstart code block byte-for-byte from
  `examples/quickstart.{py,ts}` (verified by CI; §15).

## 15. Quickstart

Six-line demo per language:

```python
client = SealStack.bearer(url="http://localhost:7070", token="dev-token")
client.admin.schemas.register(meta=compiled_schema)
client.admin.schemas.apply_ddl(qualified="examples.Doc", ddl=ddl)
client.admin.connectors.register(kind="local-files",
                                 schema="examples.Doc",
                                 config={"root": "./docs"})
client.admin.connectors.sync("local-files/examples.Doc")
result = client.query(schema="examples.Doc", query="getting started")
```

Lives in `sdks/python/examples/quickstart.py` and
`sdks/typescript/examples/quickstart.ts`. Linked from each SDK's
README and from the docs site's Quickstart page.

**Two CI guarantees**:

1. **Quickstart tests are release blockers**, not smoke tests. A
   failing Quickstart cannot ship; either it's fixed or the change
   that broke it is reverted. Runs as part of the integration job.
2. **README byte-for-byte equality.** The README's Quickstart code
   block matches `examples/quickstart.{py,ts}` byte-for-byte. CI
   script extracts the README's first fenced code block and diffs
   against the example file. Prevents the common drift pattern
   where README and example go out of sync.

## 16. Open questions

None at spec time. All eight brainstorm questions closed (see
brainstorm transcript). New questions surface during plan-writing
or review.
