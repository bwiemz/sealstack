# Roadmap

See [`design/SealStack-Plan.md`](./design/SealStack-Plan.md) §10 for the detailed
phase plan. Current milestone: **v0.3.0 — public OSS launch** (in flight).

| Milestone | Target | Status |
|-----------|--------|--------|
| v0.1.0-scaffold — repo builds + unit tests green | Month 1 | shipped |
| v0.2.0 — private alpha to design partners | Month 2 | shipped |
| v0.3.0 — public Apache-2.0 launch | Month 5 | **in flight** |
| v0.4.0 — managed cloud (Developer + Team tiers) | Month 8 | planned |
| v1.0.0 — Enterprise Edition | Month 12 | planned |

## Honest v0.1 status

Capabilities that **work today** (build-green, unit-test-covered):

- **Engine**: hybrid BM25 + vector retrieval scoped by `tenant`; schema
  persistence + hydrate-on-boot; receipts; freshness decay; chunked-fields
  column mapping in BM25 SQL.
- **Gateway**: REST (`/v1/schemas`, `/v1/connectors`, `/v1/query`,
  `/v1/receipts`); MCP 2025-11-25 streamable HTTP at `/mcp/:server`; OAuth 2.1
  bearer middleware (HS256 dev mode); OAuth protected-resource metadata.
- **CSL compiler**: Postgres DDL (with auto `tenant` column + index), MCP
  tool descriptors, JSON schema meta.
- **Policy**: real `wasmtime`-backed `WasmPolicy` with a documented ABI,
  tested end-to-end against inline WAT fixtures. Gateway selects it via
  `SEALSTACK_POLICY_DIR`.
- **Embedder / reranker selection** via env: stub / OpenAI / Voyage /
  HTTP rerank.
- **Console**: SvelteKit admin UI — overview, schemas, connectors, query,
  receipts, settings. Reads + writes through the gateway's REST surface.
- **Docs site**: Astro + Starlight with custom industrial-editorial theme,
  deployed to Cloudflare Pages.
- **TypeScript SDK** (`@sealstack/client`, `sdks/typescript/`) — full
  client implementation, generated types from the api-types JSON Schemas,
  msw-mocked unit tests, parametrized fixture corpus tests, and a
  smoke suite that runs against a live gateway in the integration job.
- **Python SDK** (`sealstack`, `sdks/python/`) — same surface as the TS
  SDK plus an async-context-manager protocol, sync facade, parametrized
  fixture tests via respx, and integration smoke suite. Pydantic v2
  models are generated from the same JSON Schemas as the TS types.
- **Wire contracts** (`crates/sealstack-api-types/`, `contracts/`) — single
  source of truth for the gateway REST surface, JSON Schemas emitted via
  schemars, fixture corpus consumed by both SDKs, and an `emit-fixtures`
  validator that round-trips every fixture through the typed Rust
  `Envelope<T>` so the corpus and the wire types cannot drift silently.

Capabilities that are **present but scoped** — working, but not complete:

- **Postgres scrape connector** — single-table SELECT with identifier
  allowlist, per-row body cap, lazy pool, 8-conn pool cap. Scoped out
  for v0.4: joins, LISTEN/NOTIFY streaming, full row-as-record
  projection.
- **Web (HTTP) connector** — fixed URL list (no crawl) with SSRF defense:
  scheme allowlist, pre-fetch DNS check rejecting private / loopback /
  link-local / unique-local v6 / multicast / broadcast / unspecified /
  documentation IPs; literal-IP short-circuit; html2text extraction.
  Scoped out for v0.4: crawling, robots.txt, link-graph following.
- **Notion connector** — PAT auth, `/v1/search` for pages, top-level
  block text flattening, `Notion-Version: 2022-06-28` pinned. Scoped
  out for v0.4: nested block trees, database row projection beyond
  title, OAuth.
- **GitHub connector** — PAT auth, REST pagination, emits READMEs + issues
  per repository. Scoped out for v0.4: pull requests, discussions, comments,
  GitHub App installation tokens.
- **Slack connector** — bot-token auth, `conversations.history` pagination,
  emits channel messages. Scoped out for v0.4: thread replies, file
  attachments, DMs/MPIMs (privacy — require opt-in flag).
- **Integration tests** — [`crates/sealstack-gateway/tests/end_to_end.rs`](./crates/sealstack-gateway/tests/end_to_end.rs)
  exercises the full happy path (register schema → register connector →
  sync → query). The tests stay `#[ignore]`-gated for `cargo test
  --workspace` so they don't break local-dev runs without Postgres, but
  they run unconditionally in the `integration` CI job (which provisions
  Postgres + Qdrant service containers). Opt in locally via
  `SEALSTACK_DATABASE_URL=... cargo test -- --ignored`.
- **CI** — `cargo check` + unit tests run on every PR. The `integration`
  job provisions Postgres + Qdrant, runs the gateway's `end_to_end.rs`
  ignored tests in-process, then boots the gateway binary and runs both
  SDKs' smoke suites against it. No outstanding CI plumbing for v0.2.

What's **deliberately stubbed** — scaffolding only, not functional:

- **Go SDK** (`sdks/go/`) — README only; no Go source.
- **Helm chart** (`deploy/helm/sealstack/`) — `Chart.yaml` + empty
  `templates/`. Needs resources, probes, ConfigMap for env, and
  secrets handling.
- **Terraform modules** (`deploy/terraform/{aws,gcp,azure}/`) — READMEs
  only.

## Known gaps before v0.4

- **Cedar ABAC adapter** — Phase 1 plan called for a Cedar policy backend
  alongside the WASM one. WASM lands in v0.3; Cedar is a separate
  `sealstack-policy-cedar` crate slotted for v0.4.
- **Onyx-parity chat console route** — the SvelteKit console covers
  overview, schemas, connectors, query, receipts, settings. A chat-style
  surface is deferred to v0.4.
- **Connector breadth past seven** — v0.3 ships GitHub, Slack, Google
  Drive, local-files, Postgres, Web, Notion (seven of the twelve in
  the original Phase 1 plan). Linear, Confluence, Jira, Gmail, S3 are
  v0.4 candidates.
- **Semantic-chunking improvements** — current chunker is a dependency-free
  approximation; needs a real tokenizer (e.g. `tiktoken-rs`) for token-
  budget fidelity.
- **Vector-store filter DSL** — retrieval currently threads only the
  `tenant` key; generic facet filters need mapping into each backend's
  native filter language.
- **Helm + Terraform** — chart skeleton and module READMEs exist; full
  resource specs land with v0.4 managed-cloud delivery.

If something listed above as "works" does not appear to work on your
install, that's a bug. Please file it at
[github.com/bwiemz/sealstack/issues](https://github.com/bwiemz/sealstack/issues).
