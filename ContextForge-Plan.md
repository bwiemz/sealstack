# ContextForge

**The open, developer-first context operating system for enterprises.**
MCP-native · Self-hostable · Typed context schemas

---

## 1. Executive Summary

Enterprise AI is powerful but structurally blind — it sees prompts, not the organization. Every deployed agent, copilot, and RAG pipeline re-solves the same four problems from scratch: how to ingest messy systems, how to preserve permissions, how to decide *what to retrieve*, and how to prove the answer is grounded. MIT cites that ~95% of enterprise AI pilots never reach production, and the failures cluster around context, not model quality.

The incumbent response is a closed, expensive layer — Glean (~$50/user/mo + $15 AI add-on, $50K+ minimums, $70K paid POCs, 10% support fees, 7-12% annual renewal hikes, fully-loaded TCO $350K–$480K/yr). Glean's valuation is $7.2B, so the market is real. The open-source response (Onyx, PipesHub, Dust) has proven the demand for self-hosted, transparent, permissive-licensed alternatives but is largely search-first and Python-heavy.

**ContextForge** is the third thing: an open-core *context operating system* — infrastructure that unifies ingestion, retrieval, memory, policy, and tool-use under a single typed context model, exposes every internal system as a governed MCP server, and ships both a free self-hostable core and a managed commercial tier. It is purpose-built for the 2026 reality where every AI system speaks MCP and every company needs the same five-layer context stack.

The strategy mirrors your QuantForge pattern: two-repo open-core, Apache-2.0 engine, commercial EULA for premium modules, with a Rust/Axum + SvelteKit stack that inherits the same deployment and packaging story.

---

## 2. Market Opportunity

### The problem in one paragraph

Enterprises run on 50–200 SaaS apps, private databases, wikis, ticketing systems, and proprietary codebases. AI agents cannot act reliably without a live, permissioned, semantically-coherent view of this data. Today that view is reconstructed ad-hoc for every use case — a separate RAG pipeline per team, a separate connector per model, a separate permission mapping per deployment. The result is duplicated infrastructure, stale context, silent permission leaks, and hallucinations that keep AI stuck in pilot hell.

### Why now

1. **MCP is no longer optional.** Anthropic launched MCP in Nov 2024; by March 2026 all major providers (OpenAI, Google, Microsoft, AWS, Cloudflare) ship native support. Monthly SDK downloads grew from 2M → 68M in ~18 months. Gartner projects 75% of API gateway vendors and 50% of iPaaS vendors will ship MCP features by end of 2026, and 40% of enterprise apps will embed task-specific agents (up from <5% today).
2. **"Context platforms" got named as a category.** Theory VC called it out in Oct 2025 as distinct from LLM providers, orchestration frameworks, and RAG tools. Analysts (Info-Tech, HyperFRAME) are explicitly framing this as the "AI operating layer" race — ServiceNow's Context Engine, OutSystems' Enterprise Context Graph, Atlan's governance layer, and Contextual AI's unified context layer are all staking claims.
3. **The open-source lane is under-served.** Onyx (MIT, ex-Danswer) and PipesHub are the only real OSS platforms, and both are primarily search interfaces. Dust is MIT but is an agent builder. None of them ship as a true MCP-native context OS with typed schemas and developer-first DX.

### Competitive landscape

| Platform | Layer(s) | Model | Pricing | Gap ContextForge exploits |
|---|---|---|---|---|
| **Glean** | Retrieval + search UX | Closed SaaS | $50/user + $15 AI, $50K min, $70K POC | Opacity, TCO, closed, no self-host |
| **Atlan** | Governance / metadata | Closed SaaS | Custom enterprise | Governs context but doesn't serve it at runtime |
| **Contextual AI** | Retrieval + agents | Closed SaaS | Custom | Vertical focus, closed |
| **ServiceNow Context Engine** | Governance + workflow | Closed, platform-locked | Bundled | Requires ServiceNow dependency |
| **OutSystems Context Graph** | App platform | Closed | Bundled | Requires OutSystems |
| **Onyx (ex-Danswer)** | Retrieval + chat | MIT OSS | Free + paid cloud ~$16/user | Python-heavy, search-first, no typed context |
| **PipesHub** | Retrieval + workflow | OSS | Free + paid | Early-stage, narrow connectors |
| **Dust** | Agent builder | MIT OSS | Paid cloud | Not infrastructure, app-layer |
| **LangChain / LangGraph** | Orchestration framework | MIT OSS | LangSmith $39/mo | Framework, not a platform; DIY governance |
| **LlamaIndex** | Retrieval framework | MIT OSS | LlamaCloud credits | Framework, not a platform |
| **Pinecone / Weaviate / Vespa** | Vector DB | Varies | Usage-based | Infrastructure primitive, not a context layer |

**The wedge:** there is no opinionated, open-core, MCP-native, self-hostable *platform* that covers all five context layers (ingestion, memory, retrieval, policy, tooling) with first-class developer DX and typed context schemas. That is the slot ContextForge fills.

---

## 3. Product Vision

**A single platform where every piece of a company's context — documents, tables, chats, tickets, code, decisions — is ingested, typed, permissioned, versioned, retrievable, and exposed to any MCP-compatible AI client, locally or in the cloud, under the customer's control.**

### Three pillars

1. **Typed Context.** Context Schema Language (CSL) lets teams declare the shape of their institutional knowledge — entities, relations, permissions, lineage — the same way they declare API types. Every context item flowing through the system is validated, versioned, and queryable.
2. **MCP-native.** ContextForge is an MCP host *and* auto-generates MCP servers from connectors, schemas, and policies. Any AI client (Claude, ChatGPT, Cursor, Gemini, internal agents) can consume context without custom integration.
3. **Open-core, local-first.** The engine is Apache-2.0 and runs on a laptop, a single VM, or a Kubernetes cluster. Premium modules (advanced governance, enterprise SSO, premium connectors, cluster mode) ship under a commercial EULA identical in structure to QuantForge's.

### What ContextForge is **not**

- Not a chat UI (the engine exposes APIs and MCP; a reference SvelteKit console ships separately and is replaceable).
- Not a vector database (it orchestrates one — pluggable across Qdrant, LanceDB, pgvector, Vespa).
- Not an LLM (model-agnostic, BYOK for every provider).
- Not a workflow/agent builder (it feeds them; LangGraph, CrewAI, and custom agents plug in over MCP).

---

## 4. Architecture

Five-layer stack, each layer independently deployable:

```
 ┌─────────────────────────────────────────────────────────────┐
 │  L5 · Surface         MCP Gateway · REST · GraphQL · CLI     │
 ├─────────────────────────────────────────────────────────────┤
 │  L4 · Policy          RBAC · ABAC · PII · Lineage · Audit    │
 ├─────────────────────────────────────────────────────────────┤
 │  L3 · Retrieval       Hybrid search · rerank · graph walk    │
 ├─────────────────────────────────────────────────────────────┤
 │  L2 · Memory          Typed store · vectors · KG · episodes  │
 ├─────────────────────────────────────────────────────────────┤
 │  L1 · Ingestion       Connectors · CDC · parsers · chunkers  │
 └─────────────────────────────────────────────────────────────┘
```

### L1 — Ingestion

- 60+ first-party connectors at GA (Slack, Gmail, GDrive, Notion, Confluence, Jira, Linear, GitHub, GitLab, Salesforce, HubSpot, Zendesk, Intercom, Postgres/MySQL/Snowflake/BigQuery, S3/GCS/Azure Blob, Sharepoint, OneDrive, Teams, Outlook, Box, Dropbox, Airtable, Asana, ClickUp, PagerDuty, Datadog, Sentry, Figma, Miro, Loom).
- Connector SDK (`contextforge-connector-sdk` in Rust and TypeScript) with a 200-line minimum contract: `list()`, `fetch()`, `subscribe()`, `map_permissions()`.
- CDC-first where available (webhooks, event streams); polling with adaptive backoff as fallback.
- Parsers for PDF (with layout), DOCX, PPTX, HTML, Markdown, source code (tree-sitter per language), audio (Whisper-pluggable), images (CLIP-pluggable).
- Chunking strategies: fixed-token, recursive, semantic (via embeddings), layout-aware (for PDFs/slides), AST-aware (for code). Chunking strategy is declared in the Context Schema, not hardcoded.

### L2 — Memory

- **Typed context store** on Postgres + pluggable vector store (Qdrant default, pgvector, LanceDB, Vespa, Weaviate).
- **Knowledge graph** layer built on Postgres (graph-as-tables with recursive CTEs) or optional Neo4j/Kùzu backend. Entities, relations, and facts extracted via configurable pipelines.
- **Episodic memory** for agent sessions — conversations, tool calls, outcomes — queryable alongside document context.
- **Versioning & freshness decay**: every chunk carries source version, last-touched timestamp, and a TTL; retrieval ranking uses configurable decay functions (borrowing the pattern Vespa/Onyx use).

### L3 — Retrieval

- Hybrid: dense (configurable embedder — OpenAI, Cohere, Voyage, Nomic, local BGE/E5) + sparse (BM25) + reranker (Cohere rerank, bge-reranker, or self-hosted).
- **Context Assembly Engine**: given a query + caller identity + token budget, walks the knowledge graph, pulls ranked chunks, dedupes, and packs a typed response that fits the budget. This is the core runtime primitive — everything above it (MCP tools, REST endpoints) is a thin wrapper.
- Per-query configurability via a small DSL (the "context query" syntax) or the REST API. Defaults are production-sane.
- Multi-tenant namespacing with strict isolation (separate schemas per tenant in Postgres, collection-per-tenant in vector store).

### L4 — Policy

- **Permission inheritance**: connectors emit permission predicates at ingest; every retrieval applies the caller's identity to filter results *before* they reach the LLM. This is the single biggest source of silent data leaks in naive RAG stacks — it is a first-class primitive, not an add-on.
- **Policy-as-code** via a WASM-hosted evaluator (choice of Rego/OPA, Cedar, or a built-in expression language). Policies are declared in CSL and compile to executable predicates.
- **PII detection and redaction** at ingest and/or query time (Presidio-compatible, pluggable).
- **Lineage**: every chunk carries a full provenance graph from source → parse → chunk → embed → retrieve. Every answer ContextForge serves can produce a signed receipt of which sources contributed, which policies applied, and which caller asked.
- **Audit log** as an append-only event stream, exportable to SIEM (Splunk, Datadog, CloudWatch).

### L5 — Surface

- **MCP Gateway**: auto-generates an MCP server per Context Schema, exposing typed tools (`search_docs`, `find_customer`, `get_incident_context`, etc.) that any MCP client consumes. Handles OAuth 2.1, session management, streamable HTTP, and SEP-current auth conformance.
- **REST + GraphQL APIs** for non-MCP clients.
- **CLI** (`cfg`) for schema CRUD, connector ops, query testing, and deployment.
- **SDKs**: Rust (canonical), TypeScript/Python/Go (derived).
- **Reference Console** (SvelteKit) for admin, exploration, and non-developer users. Replaceable — the gateway is the product.

---

## 5. Signature Features

These are the things no incumbent ships today and that form the concrete marketing story.

### 5.1 Context Schema Language (CSL)

A typed DSL for declaring context shapes. Example:

```csl
schema Customer {
  id: Ulid @primary
  name: String @searchable
  tier: Enum("free","pro","enterprise") @facet
  owner: Ref<User> @permission.read = (caller.team == self.owner.team)
  health_score: F32? @computed("analytics.customer_health")
  relations {
    tickets: many Ticket via Ticket.customer
    contracts: many Contract via Contract.customer
  }
  context {
    chunking = semantic(max_tokens=512)
    embedder = "voyage-3"
    freshness_decay = exponential(half_life = 30d)
  }
}
```

CSL files live in the customer's repo, ship with their infra-as-code, compile to migrations, and drive both retrieval behavior and the auto-generated MCP tools. This is the single most defensible differentiator — it turns context from an operations problem into a type-system problem, which is the right abstraction.

### 5.2 Auto-generated MCP servers

Every registered schema produces an MCP server at `/mcp/<schema>` with typed tools derived from the schema's queries and permissions. Customers configure their agents once and never write connector glue again. Exposes SEP-compliant OAuth 2.1, per-tool RBAC, rate limits, and audit emission out of the box.

### 5.3 Grounded answer receipts

Every response produced through ContextForge ships with a verifiable receipt: the caller identity, the policies that applied, the sources retrieved (with versions and timestamps), and the LLM's final answer. Receipts are content-addressed and optionally signed. This is the audit-trail/compliance story that enterprise security teams are actively buying.

### 5.4 Local-first mode

`cfg dev` stands up the entire stack on a laptop with Docker Compose — Postgres, Qdrant, the engine, and the console — in under 60 seconds. No cloud dependency. Drop an API key in, point at a GitHub repo and a Slack workspace, and you have a working context layer the same afternoon. This is the on-ramp the closed incumbents structurally cannot offer.

### 5.5 Shadow mode for migration

Import existing RAG pipelines (LangChain, LlamaIndex, raw Pinecone collections) and run ContextForge alongside them in "shadow mode" — it replays queries, compares retrieval quality, and emits a delta report. This turns the migration conversation from a full-platform rewrite into an A/B evaluation.

### 5.6 Context freshness guarantees

SLOs on how stale context can be per source type, enforced by the ingestion layer. Stale context is flagged at retrieval time (`freshness: stale` in response metadata), and optionally excluded. No other platform ships this as a first-class primitive.

---

## 6. Tech Stack

Consistent with the QuantForge stack where it makes sense, with deliberate choices for the few new constraints.

### Backend — Engine (Rust)

- **Language**: Rust (2024 edition, MSRV 1.83+)
- **HTTP**: Axum 0.8 + Tower middleware
- **Async runtime**: Tokio with the multi-threaded scheduler
- **Database**: PostgreSQL 16+ via `sqlx` (compile-time checked queries)
- **Migrations**: `sqlx migrate` with forward + down migrations
- **Vector store**: Qdrant (default, via `qdrant-client`); pluggable trait for LanceDB, pgvector, Vespa, Weaviate
- **Search**: Tantivy for in-process BM25 where sensible; Meilisearch optional
- **Embeddings**: pluggable trait; ships adapters for OpenAI, Cohere, Voyage, Nomic, and local inference via `candle` (bge-*, e5-*, jina-*)
- **Auth**: `oauth2` + `openidconnect` crates; OAuth 2.1 / PKCE / DPoP ready per MCP 2026 spec
- **Policy engine**: WASM runtime via `wasmtime`; reference policies authored in Cedar and compiled to WASM
- **Queue / jobs**: `apalis` on Redis for ingestion jobs; CDC consumers as long-running tasks
- **Observability**: `tracing` + OpenTelemetry exporter, Prometheus metrics, structured JSON logs
- **Testing**: `insta` for snapshot tests, `criterion` for benchmarks, `testcontainers` for integration

Rationale: same toolchain as QuantForge means single build system, shared CI, and you don't re-learn async patterns. Rust is also the right choice for a gateway that will run in front of retrieval hot paths — p99 latency on retrieval is a hard differentiator against Python-heavy incumbents (Onyx, PipesHub, Dust).

### Frontend — Console (SvelteKit)

- **Framework**: SvelteKit 2 with Svelte 5 (runes)
- **Styling**: Tailwind 4 + shadcn-svelte
- **State**: TanStack Query for server state, URL-first routing
- **Charts**: layercake or uPlot for admin metrics
- **Auth**: OIDC via the engine's gateway; no separate identity stack in the frontend

### Connector SDK

- Canonical SDK in Rust; TypeScript SDK auto-generated from trait definitions via `ts-rs`
- Python SDK as a thin wrapper around the REST API (for data-engineering teams who live in Python)

### Ingestion runtime

- Rust workers for high-throughput sources (Postgres CDC, S3, Git)
- Node.js / Python workers for SaaS connectors where vendor SDKs are JS/Python-native — isolated as separate binaries, communicated with over gRPC

### CLI (`cfg`)

- Rust, built on `clap` v4 with derive + `indicatif` for progress
- Ships as a single static binary for macOS (arm64/x86_64), Linux (arm64/x86_64), Windows (x86_64)

### Packaging & deployment

- Docker images (distroless where possible)
- Official Helm chart for Kubernetes
- Terraform modules for AWS/GCP/Azure reference deployments
- `cfg dev` single-command local bring-up via Docker Compose

### CI / dev tooling

- GitHub Actions (matrix across Linux/macOS/Windows, stable + beta Rust)
- `cargo-deny`, `cargo-audit`, `cargo-semver-checks` on every PR
- SBOM generation (CycloneDX) on release
- Conventional Commits + `release-please` for automated changelogs and versioned releases

### Model / inference integration

- BYOK for every frontier provider (Anthropic, OpenAI, Google, Mistral, Cohere)
- Local inference via `candle` for embedders, `llama.cpp` or `vllm` (external) for LLMs
- Token accounting and per-tenant budgets as first-class primitives

---

## 7. Pricing & Tiers

Transparent pricing is one of the top three buyer complaints about Glean — every comparison article leads with it. ContextForge ships public pricing from day one.

### Tier 1 — **Community** (Free, self-hosted, OSS)

- Apache-2.0 engine, console, CLI, SDKs
- Up to 3 workspaces, unlimited users
- Core connectors: GitHub, GDrive, Notion, Slack, Postgres, S3, Web (~12 connectors)
- BYOK for all model providers
- Community support (Discord + GitHub Discussions)
- All L1–L5 primitives: ingestion, memory, retrieval, basic policy (RBAC), MCP gateway
- SQLite-backed dev mode for laptops; Postgres + Qdrant for production
- Runs on a single VM or a laptop
- **Intent**: be the default local-first choice for solo devs, teams, and small orgs; drive adoption

### Tier 2 — **Developer Cloud** ($0 base, usage-metered)

- Same feature set as Community, hosted by us
- Free allowance: 1 GB indexed content, 100K retrieval queries/mo, 1 M embedding tokens/mo
- Overage: $0.15/GB-month storage, $0.50 per 1K retrieval queries, embedding passthrough at 1.1× provider cost
- GitHub OAuth sign-in, single workspace
- Community support
- **Intent**: zero-friction onboarding for individual developers and hackathons

### Tier 3 — **Team** ($39/user/month, billed annually; $49 monthly)

- Everything in Developer Cloud, plus:
- Up to 50 users
- Extended connector set (~40 connectors including Salesforce, HubSpot, Jira, Linear, Confluence, Zendesk, Intercom)
- Google / Microsoft / Okta SSO
- ABAC policies beyond basic RBAC
- Lineage + grounded answer receipts
- SOC 2 Type II certified infrastructure
- Email support, 1 business day SLA
- **Intent**: land-and-expand tier; sits well below Glean's $50 base and $15 AI add-on while delivering more

### Tier 4 — **Business** ($69/user/month, billed annually; 50-500 users)

- Everything in Team, plus:
- Full connector catalog (60+)
- Custom connectors via SDK with our review
- Dedicated vector store tier with reserved capacity
- VPC peering (AWS PrivateLink, GCP Private Service Connect)
- BYOK encryption (customer-managed KMS keys)
- Advanced policy features: Cedar policies, PII redaction, data residency controls
- 99.9% uptime SLA with credits
- Priority support, 4-hour response SLA, dedicated Slack channel
- **Intent**: the Glean displacement tier — undercut at comparable feature parity

### Tier 5 — **Enterprise** (Custom, 500+ users, or regulated deployments)

- Everything in Business, plus:
- **Self-hosted Enterprise Edition** — run the full stack in your VPC, air-gapped deployments supported
- FedRAMP-ready deployment pattern (target; certification on roadmap)
- HIPAA BAA, ISO 27001, SOC 2 Type II
- Dedicated solutions engineering, implementation services
- Custom connectors as a service
- Cluster mode (horizontal scaling, multi-region, active-active)
- Premium governance modules (described in §9)
- 24/7 support with named TAM, 1-hour response SLA
- Contract floor: $60K ARR; typical deployments $120K–$500K ARR
- **Intent**: the direct Glean enterprise replacement; pricing published as "starts at $60K" for transparency, not a wall

### Add-ons (all tiers Team and above)

- **Additional regions**: +$500/mo per region for managed tiers
- **Premium connectors** (SAP, Oracle, Workday, NetSuite, Greenhouse, custom ERPs): $200–$500/mo each or bundled in Enterprise
- **Extended retention of lineage receipts**: storage-based

### Revenue model rationale

- Per-seat pricing is the industry standard buyers expect; don't fight that.
- But publish every number. Opacity is actively losing deals for Glean right now — every competitive comparison article leads with "Glean requires a sales call."
- Usage-metered Developer tier drives bottoms-up adoption (Dust/Onyx/Linear playbook).
- Enterprise tier is where most revenue lives; the free + Team tiers are lead generation.

---

## 8. Licensing Strategy

Identical two-repo pattern to QuantForge.

### Public repo — `contextforge/contextforge`

- **License**: Apache-2.0 with a narrow `CLA.md` (standard Individual + Corporate CLA)
- **Contents**: engine core, connector SDK, CLI, console, standard connectors, CSL compiler, reference Docker/Helm/Terraform
- **Why Apache-2.0 (not MIT)**: patent grant clause. ContextForge sits adjacent to prior art in search, retrieval, and policy evaluation — explicit patent grants protect contributors and users. MIT doesn't. Dust/Onyx picked MIT; we can differentiate on this.
- **Why not AGPL/SSPL/BUSL**: the category is still being defined. Permissive licensing is the fastest way to become the default, and you capture value through the commercial repo, managed service, and premium modules — not by forcing upstream contribution.

### Private repo — `contextforge/contextforge-enterprise`

- **License**: ContextForge Commercial License (EULA) — template derived from your QuantForge EULA
- **Contents**:
  - Premium connectors (SAP, Oracle, Workday, NetSuite, ServiceNow, premium security tools)
  - Cluster mode / horizontal scaling coordination (consensus, sharding, multi-region replication)
  - Enterprise SSO extensions (SCIM provisioning, advanced SAML attribute mapping)
  - Advanced governance modules (PII detection beyond Presidio baseline, DLP hooks, compliance pack generators)
  - FedRAMP / HIPAA / ISO hardening modules
  - Managed cloud control plane code
- **Distribution**: precompiled binaries to paid customers; source access under NDA for Enterprise tier at customer request
- **Rationale**: this is the premium feature set that enterprises will pay for specifically and that is not valuable to open-source by itself. It does not cripple the OSS product.

### CLA

- Individual CLA and Corporate CLA (Apache-style), managed via `cla-assistant.io` bot on the public repo.
- Grants a perpetual, irrevocable, non-exclusive license to redistribute contributions under Apache-2.0 *and* re-license into the commercial product. This is the standard open-core CLA pattern and is non-controversial at this scale.

### Trademark

- Register "ContextForge" wordmark and logo (class 9 software, class 42 SaaS).
- `TRADEMARKS.md` in the public repo: the code is Apache-2.0, the name and logo are not. Forks must rebrand (the standard approach — see MongoDB, HashiCorp, Elastic).

### Third-party licenses

- `THIRD_PARTY_NOTICES.md` auto-generated on every release via `cargo about` and `pnpm licenses`.
- SBOM (CycloneDX format) attached to every GitHub release.

---

## 9. GitHub File Structure

### Public repo: `contextforge/contextforge`

```
contextforge/
├── .github/
│   ├── workflows/
│   │   ├── ci.yml                    # lint, test, build matrix
│   │   ├── release.yml               # release-please, cargo publish, npm publish
│   │   ├── docker.yml                # multi-arch image builds
│   │   ├── security.yml              # cargo-audit, cargo-deny, trivy
│   │   └── docs.yml                  # deploy docs site
│   ├── ISSUE_TEMPLATE/
│   ├── PULL_REQUEST_TEMPLATE.md
│   ├── CODEOWNERS
│   └── dependabot.yml
│
├── crates/
│   ├── cfg-engine/                   # core runtime (L2+L3+L4)
│   │   ├── src/
│   │   │   ├── memory/               # typed store, vector, KG
│   │   │   ├── retrieval/            # hybrid search, assembly
│   │   │   ├── policy/               # WASM policy eval
│   │   │   └── lib.rs
│   │   ├── tests/
│   │   └── Cargo.toml
│   ├── cfg-gateway/                  # L5: HTTP + MCP surface
│   │   ├── src/
│   │   │   ├── mcp/                  # MCP server impl
│   │   │   ├── rest/                 # REST endpoints
│   │   │   ├── graphql/              # GraphQL schema
│   │   │   └── auth/                 # OAuth 2.1 + OIDC
│   │   └── Cargo.toml
│   ├── cfg-ingest/                   # L1: connector runtime
│   │   ├── src/
│   │   │   ├── parsers/              # PDF, DOCX, code, etc.
│   │   │   ├── chunkers/             # fixed, semantic, AST
│   │   │   └── scheduler/            # jobs, CDC, polling
│   │   └── Cargo.toml
│   ├── cfg-csl/                      # Context Schema Language
│   │   ├── src/
│   │   │   ├── parser/               # grammar (lalrpop or winnow)
│   │   │   ├── ast/
│   │   │   ├── types/                # type checker
│   │   │   ├── codegen/              # SQL migrations, TS types, MCP descriptors
│   │   │   └── lib.rs
│   │   └── Cargo.toml
│   ├── cfg-connector-sdk/            # Rust connector trait + helpers
│   ├── cfg-embedders/                # embedder adapters
│   ├── cfg-vectorstore/              # vector store trait + adapters
│   ├── cfg-cli/                      # `cfg` binary
│   ├── cfg-common/                   # shared types, errors, telemetry
│   └── cfg-receipts/                 # grounded answer receipts
│
├── connectors/
│   ├── README.md                     # how to write a connector
│   ├── slack/
│   ├── github/
│   ├── gdrive/
│   ├── notion/
│   ├── postgres/
│   ├── s3/
│   ├── web/
│   └── ... (12 standard connectors)
│
├── sdks/
│   ├── typescript/                   # @contextforge/sdk
│   │   ├── src/
│   │   ├── package.json
│   │   └── tsconfig.json
│   ├── python/                       # contextforge (PyPI)
│   │   ├── contextforge/
│   │   ├── pyproject.toml
│   │   └── tests/
│   └── go/                           # github.com/contextforge/go-sdk
│
├── console/                          # SvelteKit admin UI
│   ├── src/
│   │   ├── routes/
│   │   ├── lib/
│   │   └── app.html
│   ├── static/
│   ├── package.json
│   ├── svelte.config.js
│   └── tailwind.config.ts
│
├── deploy/
│   ├── docker/
│   │   ├── Dockerfile.engine
│   │   ├── Dockerfile.gateway
│   │   ├── Dockerfile.ingest
│   │   └── compose.dev.yaml          # `cfg dev` stack
│   ├── helm/
│   │   └── contextforge/
│   │       ├── Chart.yaml
│   │       ├── values.yaml
│   │       └── templates/
│   └── terraform/
│       ├── aws/
│       ├── gcp/
│       └── azure/
│
├── examples/
│   ├── customer-360/                 # CRM + tickets + calls
│   ├── engineering-context/          # GitHub + Linear + Slack + Sentry
│   ├── legal-dd/                     # contracts + matters
│   └── README.md
│
├── docs/                             # docs site (Astro Starlight or MkDocs)
│   ├── src/
│   │   ├── content/
│   │   │   ├── getting-started/
│   │   │   ├── concepts/             # context schemas, retrieval, policy
│   │   │   ├── guides/
│   │   │   ├── reference/            # CLI, API, CSL, SDK
│   │   │   ├── connectors/
│   │   │   └── deployment/
│   │   └── ...
│   └── astro.config.mjs
│
├── scripts/
│   ├── bootstrap.sh
│   ├── release.sh
│   └── generate-third-party-notices.sh
│
├── Cargo.toml                        # workspace
├── Cargo.lock
├── rust-toolchain.toml
├── pnpm-workspace.yaml
├── package.json                      # root for console + sdks/typescript
├── .cargo/config.toml
├── .rustfmt.toml
├── clippy.toml
├── deny.toml                         # cargo-deny config
├── .editorconfig
├── .gitignore
├── LICENSE                           # Apache-2.0
├── NOTICE
├── TRADEMARKS.md
├── THIRD_PARTY_NOTICES.md            # generated
├── SECURITY.md
├── CODE_OF_CONDUCT.md
├── CONTRIBUTING.md
├── GOVERNANCE.md                     # project governance, Working Groups pattern
├── CHANGELOG.md                      # release-please maintained
├── README.md
└── ROADMAP.md
```

### Private repo: `contextforge/contextforge-enterprise`

```
contextforge-enterprise/
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── release.yml
├── crates/
│   ├── cfg-cluster/                  # horizontal scaling, consensus
│   ├── cfg-premium-policy/           # advanced governance
│   ├── cfg-sso-extensions/           # SCIM, advanced SAML
│   ├── cfg-compliance/               # FedRAMP, HIPAA, ISO pack generators
│   └── cfg-pii/                      # advanced PII/DLP
├── connectors-enterprise/
│   ├── sap/
│   ├── oracle-ebs/
│   ├── workday/
│   ├── netsuite/
│   ├── servicenow/
│   ├── microsoft-purview/
│   └── ...
├── control-plane/                    # managed cloud orchestration
│   ├── crates/
│   ├── console/
│   └── deploy/
├── docs/
├── LICENSE                           # Commercial EULA
├── README.md                         # internal + entitled-customer only
└── Cargo.toml
```

---

## 10. Go-to-Market & Roadmap

### Phase 0 — Foundations (Month 0–2)

- CSL grammar spec, type checker, migration codegen
- Engine skeleton (memory + retrieval core) with Postgres + Qdrant
- Gateway skeleton with basic MCP server exposure (SEP-current)
- 6 connectors: GitHub, GDrive, Notion, Slack, Postgres, Web
- `cfg dev` single-command local bring-up
- CLI MVP (`cfg schema`, `cfg connector`, `cfg query`)
- **Ship**: private alpha to ~20 design partners

### Phase 1 — Public OSS launch (Month 3–5)

- Full 12-connector standard set
- Policy engine with RBAC + Cedar ABAC
- Grounded answer receipts
- SDK GA: TypeScript, Python
- Reference console at feature parity with Onyx chat
- Docs site complete
- **Ship**: public Apache-2.0 release + Hacker News launch + Product Hunt
- Metrics target: 2,000 GitHub stars in 90 days (Onyx trajectory is the benchmark)

### Phase 2 — Managed Cloud (Month 6–8)

- Developer + Team tiers live on cloud
- SOC 2 Type I
- Billing + metering, Stripe integration
- Expanded connector set (40 total)
- Shadow-mode RAG import tooling
- **Ship**: public cloud GA, self-service sign-up

### Phase 3 — Business & Enterprise (Month 9–12)

- VPC peering, BYOK encryption
- Enterprise Edition self-hosted (private repo)
- Full 60+ connector catalog
- Cluster mode (multi-region)
- SOC 2 Type II
- **Ship**: first 5–10 paying enterprise customers; target $1M ARR

### Phase 4 — Ecosystem (Month 13+)

- Connector marketplace (third-party connectors with revenue share)
- CSL registry (shareable schema libraries)
- Plugin/extension system for custom retrieval strategies
- FedRAMP moderate in-process
- HIPAA-attested deployments

### Positioning statement

> **"The open context OS for AI-native companies. Plug in your data, define your context in typed schemas, expose it to any MCP client — locally or in your cloud, under your control."**

### Primary ICPs

1. **Mid-market engineering orgs** (50–500 employees) building internal AI tools who are priced out of Glean and tired of gluing LangChain together.
2. **Regulated industries** (healthcare, financial services, defense) that cannot send data to Glean's cloud.
3. **AI-native startups** building customer-facing products on top of customer data — they need an enterprise-grade retrieval backbone without building it themselves.

### Content / developer marketing

- Weekly technical blog (retrieval benchmarks, CSL deep dives, connector internals)
- Detailed competitive comparisons with prices published (the exact content the Glean vs X articles do, but ours with real numbers)
- Open-source office hours
- Conference talks: MCP Summit, KubeCon, QCon, AI Engineer Summit
- Reference implementations on GitHub for visible use cases (customer-360, engineering-context, legal-dd)

---

## 11. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Glean/Atlan open-source a competing core | Medium | High | Be 18 months ahead on CSL and MCP-native; build a real community moat |
| MCP fragmentation (Google Task API competition) | Medium | Medium | Support both; MCP is already winning the cross-vendor race |
| Connector maintenance burden | High | Medium | Typed connector SDK + contract tests + marketplace + revenue share |
| Enterprise compliance debt (FedRAMP, HIPAA) | High | High | Partner with compliance-as-a-service vendors; start SOC 2 early |
| LLM provider lock-in pressure | Low | Medium | BYOK everywhere; never depend on a single provider |
| Vector DB commoditization | High | Low | Pluggable from day 1; we are the context layer, not the vector store |
| Open-core cannibalization | Medium | Medium | Draw the line at *enterprise-specific* features (cluster mode, premium connectors, compliance) — not at core quality |

---

## 12. Appendix — Numbers Worth Anchoring On

- Glean valuation: $7.2B (June 2025 Series F)
- Glean typical TCO: $350K–$480K/year fully loaded
- Glean starting cost: $50/user + $15 AI add-on, $50–60K minimum, $70K paid POC, 10% support fee
- MCP SDK monthly downloads: 2M (Nov 2024) → 68M (Nov 2025) → still growing
- LangChain: ~119K GitHub stars, $1.1B valuation (Series B 2025)
- Onyx: MIT, used by Netflix, Ramp, Thales — proof point that OSS in this category works
- Gartner: 75% of API gateway vendors and 50% of iPaaS vendors will have MCP features by end 2026
- Gartner: 40% of enterprise apps will embed task-specific agents by end 2026 (up from <5%)
- MIT: ~95% of enterprise AI pilots never reach production — framing stat for every pitch

---

*End of plan. Suggest next step: lock CSL grammar spec + ingest the first 6 connectors into a working L1→L2→L3 loop, so you have a demoable core before writing any gateway code.*
