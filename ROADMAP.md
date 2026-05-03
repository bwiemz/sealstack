# Roadmap

See [`design/SealStack-Plan.md`](./design/SealStack-Plan.md) §10 for the detailed
phase plan. Current milestone: **v0.1.0-scaffold** (repo scaffolds, builds,
and unit tests all pass).

| Milestone | Target | Status |
|-----------|--------|--------|
| v0.1.0-scaffold — repo builds + unit tests green | Month 1 | **in progress** |
| v0.2.0 — private alpha to design partners | Month 2 | planned |
| v0.3.0 — public Apache-2.0 launch | Month 5 | planned |
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

Capabilities that are **present but scoped** — working, but not complete:

- **GitHub connector** — PAT auth, REST pagination, emits READMEs + issues
  per repository. Scoped out for v0.2: pull requests, discussions, comments,
  GitHub App installation tokens.
- **Slack connector** — bot-token auth, `conversations.history` pagination,
  emits channel messages. Scoped out for v0.2: thread replies, file
  attachments, DMs/MPIMs (privacy — require opt-in flag).
- **Integration tests** — [`crates/sealstack-gateway/tests/end_to_end.rs`](./crates/sealstack-gateway/tests/end_to_end.rs)
  exercises the full happy path (register schema → register connector →
  sync → query), but is `#[ignore]`-gated because it needs a running
  Postgres. Opt in via `SEALSTACK_DATABASE_URL=... cargo test -- --ignored`.
- **CI** — `cargo check` + unit tests run on every PR. Integration tests do
  not run in CI yet (needs containerized Postgres); that wires in v0.2.

What's **deliberately stubbed** — scaffolding only, not functional:

- **Go SDK** (`sdks/go/`) — README only; no Go source.
- **Python SDK** (`sdks/python/sealstack/`) — package layout + httpx client
  skeleton; not wired against real endpoints yet.
- **TypeScript SDK** (`sdks/typescript/`) — build config + export surface;
  no client implementation.
- **Helm chart** (`deploy/helm/sealstack/`) — `Chart.yaml` + empty
  `templates/`. Needs resources, probes, ConfigMap for env, and
  secrets handling.
- **Terraform modules** (`deploy/terraform/{aws,gcp,azure}/`) — READMEs
  only.

## Known gaps before v0.2

- **Postgres-backed CI integration tests** (lift the `#[ignore]`) — in flight
  in #45.
- **Semantic-chunking improvements** — current chunker is a dependency-free
  approximation; needs a real tokenizer (e.g. `tiktoken-rs`) for token-
  budget fidelity.
- **Vector-store filter DSL** — retrieval currently threads only the
  `tenant` key; generic facet filters need mapping into each backend's
  native filter language.
- **Connector breadth** — Google Drive landed in #40; Notion, Linear,
  Confluence remain design-partner priorities but not yet scaffolded.
- **SDK client implementations** — TypeScript and Python SDKs are build
  skeletons only (`dependencies: {}`); needed for "SDK GA" in Phase 1.

If something listed above as "works" does not appear to work on your
install, that's a bug. Please file it at
[github.com/bwiemz/sealstack/issues](https://github.com/bwiemz/sealstack/issues).
