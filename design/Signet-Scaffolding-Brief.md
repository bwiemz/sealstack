# Signet — Agent-Executable Scaffolding Brief

**Target repo**: `bwiemz/signet` (public, Apache-2.0)
**Target stack**: Rust 1.83+ (Axum 0.8, Tokio, sqlx, wasmtime) · SvelteKit 2 + Svelte 5 · PostgreSQL 16 · Qdrant
**Executor**: AI coding agent (Claude Code, Cursor, Codex CLI) with Bash + filesystem access
**Completion criterion**: `signet dev` stands up the full stack in under 60 seconds on a clean machine, serves an MCP endpoint, and passes the end-to-end acceptance test in §12.

This brief is ordered. Do each phase to completion and run its verification step before moving on. If a step's verification fails, stop and report. Do not skip ahead.

---

## Prerequisites

Before running any phase, verify:

```bash
rustc --version    # >= 1.83
cargo --version
node --version     # >= 20.10
pnpm --version     # >= 9.0
docker --version
docker compose version
git --version
```

Install missing tooling before proceeding. If any prerequisite is missing, abort and report.

---

## Phase 0 — Repo Initialization

```bash
mkdir signet && cd signet
git init -b main
```

Create these root files verbatim:

**`rust-toolchain.toml`**
```toml
[toolchain]
channel = "1.83"
components = ["rustfmt", "clippy", "rust-src"]
targets = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
```

**`.rustfmt.toml`**
```toml
edition = "2024"
max_width = 100
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
reorder_imports = true
use_field_init_shorthand = true
```

**`clippy.toml`**
```toml
avoid-breaking-exported-api = false
disallowed-methods = []
```

**`deny.toml`**
```toml
[licenses]
allow = ["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-DFS-2016", "Unicode-3.0"]
confidence-threshold = 0.92

[bans]
multiple-versions = "warn"

[advisories]
yanked = "deny"
ignore = []
```

**`.editorconfig`**
```
root = true
[*]
end_of_line = lf
insert_final_newline = true
charset = utf-8
indent_style = space
indent_size = 4
[*.{ts,tsx,js,jsx,svelte,json,yaml,yml}]
indent_size = 2
[*.md]
trim_trailing_whitespace = false
```

**`.gitignore`**
```
target/
node_modules/
.svelte-kit/
dist/
.env
.env.local
*.log
.DS_Store
out/
.cargo-cache/
coverage/
```

**`LICENSE`** — the full Apache-2.0 license text (verbatim from https://www.apache.org/licenses/LICENSE-2.0.txt).

**`NOTICE`**
```
Signet
Copyright 2026 Signet Contributors

This product includes software developed by the Signet project
(https://github.com/bwiemz/signet).
```

**`TRADEMARKS.md`**
```markdown
# Trademarks

The Signet name and logo are trademarks of [Entity]. The source code in
this repository is licensed under Apache-2.0, but that license does NOT grant
permission to use the Signet name, logo, or branding. Forks and
derivative works must use a different name and mark.

See [POLICY.md](./TRADEMARK-POLICY.md) for details.
```

Commit:
```bash
git add . && git commit -m "chore: initial repo scaffold"
```

**Verify**: `ls -la` shows the files above; `git log` shows the initial commit.

---

## Phase 1 — Cargo Workspace

**`Cargo.toml`** (root)
```toml
[workspace]
resolver = "3"
members = [
    "crates/signet-common",
    "crates/signet-csl",
    "crates/signet-engine",
    "crates/signet-gateway",
    "crates/signet-ingest",
    "crates/signet-connector-sdk",
    "crates/signet-embedders",
    "crates/signet-vectorstore",
    "crates/signet-receipts",
    "crates/signet-cli",
    "connectors/*",
]
default-members = ["crates/signet-cli"]

[workspace.package]
version     = "0.1.0"
edition     = "2024"
rust-version = "1.83"
license     = "Apache-2.0"
repository  = "https://github.com/bwiemz/signet"
authors     = ["Signet Contributors"]

[workspace.lints.rust]
unsafe_code = "forbid"
unreachable_pub = "warn"
missing_docs = "warn"

[workspace.lints.clippy]
pedantic       = { level = "warn", priority = -1 }
nursery        = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_errors_doc = "allow"

[workspace.dependencies]
# Core
tokio     = { version = "1.42", features = ["full"] }
axum      = { version = "0.8", features = ["macros", "tracing"] }
tower     = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors", "compression-gzip"] }
hyper      = "1.5"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror  = "2"
anyhow     = "1"
tracing    = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
opentelemetry = "0.27"
opentelemetry_sdk = "0.27"
uuid       = { version = "1.11", features = ["v4", "v7", "serde"] }
ulid       = { version = "1.1", features = ["serde"] }
time       = { version = "0.3", features = ["serde", "parsing", "formatting"] }
# DB + storage
sqlx       = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "json", "time", "uuid"] }
qdrant-client = "1.12"
# Parser + codegen
winnow     = "0.6"
quote      = "1"
proc-macro2 = "1"
minijinja  = "2"
# Policy engine
wasmtime   = "28"
# CLI
clap       = { version = "4.5", features = ["derive", "env"] }
indicatif  = "0.17"
# Http client
reqwest    = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
# Testing
insta      = { version = "1.41", features = ["yaml"] }
testcontainers = "0.23"

[profile.dev]
split-debuginfo = "unpacked"

[profile.release]
lto = "thin"
codegen-units = 1
strip = "debuginfo"
```

Create each crate's directory and a minimal `Cargo.toml`:

```bash
for c in signet-common signet-csl signet-engine signet-gateway signet-ingest \
         signet-connector-sdk signet-embedders signet-vectorstore signet-receipts signet-cli; do
    mkdir -p crates/$c/src
    cat > crates/$c/Cargo.toml <<EOF
[package]
name        = "$c"
version     = { workspace = true }
edition     = { workspace = true }
rust-version = { workspace = true }
license     = { workspace = true }
repository  = { workspace = true }

[lints]
workspace = true

[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
EOF
    echo "//! $c crate" > crates/$c/src/lib.rs
done
```

The `signet-cli` crate needs a `[[bin]]`:

```toml
# Append to crates/signet-cli/Cargo.toml
[[bin]]
name = "cfg"
path = "src/main.rs"

[dependencies]
clap = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

Create `crates/signet-cli/src/main.rs`:
```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "cfg", version, about = "Signet CLI")]
enum Cmd {
    /// Launch the local development stack
    Dev,
    /// Manage context schemas
    Schema { #[command(subcommand)] sub: SchemaCmd },
    /// Manage connectors
    Connector { #[command(subcommand)] sub: ConnectorCmd },
    /// Execute a context query against the local engine
    Query { query: String },
    /// Compile CSL files in the current directory
    Compile,
    /// Print version and build info
    Version,
}

#[derive(clap::Subcommand, Debug)]
enum SchemaCmd {
    List,
    Validate { path: std::path::PathBuf },
    Apply { path: std::path::PathBuf },
}

#[derive(clap::Subcommand, Debug)]
enum ConnectorCmd {
    List,
    Add { name: String },
    Sync { name: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    match Cmd::parse() {
        Cmd::Dev => cmd_dev::run().await,
        Cmd::Schema { sub } => cmd_schema::run(sub).await,
        Cmd::Connector { sub } => cmd_connector::run(sub).await,
        Cmd::Query { query } => cmd_query::run(query).await,
        Cmd::Compile => cmd_compile::run().await,
        Cmd::Version => { println!("cfg {}", env!("CARGO_PKG_VERSION")); Ok(()) }
    }
}

mod cmd_dev      { pub async fn run() -> anyhow::Result<()> { Ok(()) } }
mod cmd_schema   { use super::SchemaCmd; pub async fn run(_: SchemaCmd) -> anyhow::Result<()> { Ok(()) } }
mod cmd_connector{ use super::ConnectorCmd; pub async fn run(_: ConnectorCmd) -> anyhow::Result<()> { Ok(()) } }
mod cmd_query    { pub async fn run(_q: String) -> anyhow::Result<()> { Ok(()) } }
mod cmd_compile  { pub async fn run() -> anyhow::Result<()> { Ok(()) } }
```

Run:
```bash
cargo check --workspace
cargo fmt --all
```

**Verify**: `cargo build --bin cfg` succeeds and `./target/debug/signet version` prints `cfg 0.1.0`.

Commit: `git commit -am "feat(workspace): cargo workspace and cli skeleton"`.

---

## Phase 2 — Core Traits (signet-common, signet-vectorstore, signet-embedders, signet-connector-sdk)

These crates define the abstractions every other crate depends on. Nothing else should be built until these compile.

**`crates/signet-common/src/lib.rs`**
```rust
//! Shared types, errors, and identifiers.

pub mod id;
pub mod error;
pub mod tenant;
pub mod instant;

pub use error::{SignetError, SignetResult};
pub use id::{ContextId, SchemaId, TenantId};
pub use tenant::Tenant;
```

Implement `id.rs` with newtype wrappers over `Ulid`, `error.rs` with a `thiserror` enum covering `NotFound`, `Unauthorized`, `Validation`, `Backend`, `Policy`, `Config`.

**`crates/signet-vectorstore/src/lib.rs`**
```rust
//! Vector store abstraction.

use async_trait::async_trait;
use signet_common::SignetResult;

#[derive(Clone, Debug)]
pub struct Chunk {
    pub id: ulid::Ulid,
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub id: ulid::Ulid,
    pub score: f32,
    pub content: String,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[async_trait]
pub trait VectorStore: Send + Sync + 'static {
    async fn ensure_collection(&self, name: &str, dims: usize) -> SignetResult<()>;
    async fn upsert(&self, collection: &str, chunks: Vec<Chunk>) -> SignetResult<()>;
    async fn search(
        &self,
        collection: &str,
        query_vec: Vec<f32>,
        top_k: usize,
        filter: Option<serde_json::Value>,
    ) -> SignetResult<Vec<SearchResult>>;
    async fn delete(&self, collection: &str, ids: Vec<ulid::Ulid>) -> SignetResult<()>;
}

pub mod qdrant;
pub mod memory; // in-process backend for tests
```

Add `async-trait = "0.1"`, `ulid`, `serde_json` to the crate's `Cargo.toml`. Implement `memory.rs` with a `DashMap`-backed store. Stub `qdrant.rs` with `todo!()` bodies — it is filled in Phase 5.

**`crates/signet-embedders/src/lib.rs`**
```rust
//! Embedder abstraction.

use async_trait::async_trait;
use signet_common::SignetResult;

#[async_trait]
pub trait Embedder: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn dims(&self) -> usize;
    async fn embed(&self, texts: Vec<String>) -> SignetResult<Vec<Vec<f32>>>;
}

pub mod openai;
pub mod voyage;
pub mod local;  // candle-backed; ship as a feature flag
pub mod stub;   // deterministic embedder for tests
```

Implement `stub.rs` deterministically — hash the input, expand to `dims` floats in [-1, 1]. Only stub.rs needs a real implementation in Phase 2; the others are `todo!()` stubs filled later.

**`crates/signet-connector-sdk/src/lib.rs`**
```rust
//! Connector SDK — implement this trait to add a new data source.

use async_trait::async_trait;
use signet_common::SignetResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    pub id: ResourceId,
    pub kind: String,
    pub title: Option<String>,
    pub body: String,
    pub metadata: serde_json::Map<String, serde_json::Value>,
    pub permissions: Vec<PermissionPredicate>,
    pub source_updated_at: time::OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionPredicate {
    pub principal: String,   // e.g., "user:123", "group:eng"
    pub action: String,      // "read" | "write" | "list"
}

#[async_trait]
pub trait Connector: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn version(&self) -> &str;

    /// Stream all resources the authenticated principal can see.
    async fn list(&self) -> SignetResult<Box<dyn tokio_stream::Stream<Item = Resource> + Send + Unpin>>;

    /// Fetch one resource by ID. Used for on-demand refresh.
    async fn fetch(&self, id: &ResourceId) -> SignetResult<Resource>;

    /// Subscribe to change events if the source supports push. Default: not supported.
    async fn subscribe(&self) -> SignetResult<Option<Box<dyn tokio_stream::Stream<Item = ChangeEvent> + Send + Unpin>>> {
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub enum ChangeEvent {
    Upsert(Resource),
    Delete(ResourceId),
}
```

Add `tokio-stream = "0.1"` to `signet-connector-sdk`.

Run `cargo check --workspace`. Fix any errors before continuing.

Commit: `git commit -am "feat(core): vector store, embedder, connector traits"`.

---

## Phase 3 — CSL Compiler Skeleton

**Files to create:**
- `crates/signet-csl/src/lib.rs`
- `crates/signet-csl/src/lexer.rs` (inline with parser, minimal tokens)
- `crates/signet-csl/src/parser.rs`  (uses `winnow`)
- `crates/signet-csl/src/ast.rs`
- `crates/signet-csl/src/types.rs`   (type checker)
- `crates/signet-csl/src/codegen/mod.rs`
- `crates/signet-csl/src/codegen/sql.rs`
- `crates/signet-csl/src/codegen/rust.rs`
- `crates/signet-csl/src/codegen/mcp.rs`
- `crates/signet-csl/src/codegen/vector_plan.rs`
- `crates/signet-csl/tests/parse_smoke.rs`
- `crates/signet-csl/tests/fixtures/hello.csl`

Target grammar: the subset described in §1–§6 of the CSL spec, excluding computed fields, enum variants with string forms, and policy WASM codegen (those come in Phase 6).

**Parser scope for this phase:**
- `schema X { ... }` with primitive fields and `@primary`, `@searchable`, `@indexed`, `@unique`, `@default(...)`.
- `relations { ... }` with `one`/`many` and `via`.
- `context { ... }` blocks with string/identifier/call-expression values.
- Comments.
- EBNF → `winnow` hand-written combinators.

**Type checker scope:**
- Resolve `Ref<T>` targets within a single file.
- Ensure exactly one `@primary`.
- Ensure `Vector<N>` has an `@embedded_from`.
- Emit a typed AST (`TypedFile`) consumed by codegen.

**Codegen scope:**
- `sql.rs` — emit `CREATE TABLE`, foreign keys, basic indexes.
- `rust.rs` — emit `#[derive(Serialize, Deserialize)] struct X { ... }`.
- `mcp.rs` — emit JSON tool descriptors with `search_<schema>` and `get_<schema>`.
- `vector_plan.rs` — emit YAML describing collections.

**Snapshot test**:

`crates/signet-csl/tests/fixtures/hello.csl`:
```csl
schema Note {
    id:         Ulid    @primary
    title:      String  @searchable
    body:       Text    @chunked
    created_at: Instant @default(now())

    context {
        chunking    = semantic(max_tokens = 512)
        embedder    = "stub"
        vector_dims = 64
    }
}
```

`crates/signet-csl/tests/parse_smoke.rs`:
```rust
use signet_csl::{compile, CompileTargets};

#[test]
fn hello_compiles_to_all_targets() {
    let src = include_str!("fixtures/hello.csl");
    let out = compile(src, CompileTargets::all()).expect("compile");
    insta::assert_yaml_snapshot!("hello.sql", out.sql);
    insta::assert_yaml_snapshot!("hello.rust", out.rust);
    insta::assert_yaml_snapshot!("hello.mcp", out.mcp_tools);
    insta::assert_yaml_snapshot!("hello.vector", out.vector_plan);
}
```

Run `cargo test -p signet-csl`, then `cargo insta review` and accept the snapshots. Commit the snapshot files.

Commit: `git commit -am "feat(csl): parser, type checker, and codegen skeleton"`.

---

## Phase 4 — Engine (Memory + Retrieval)

**`crates/signet-engine/src/lib.rs`**
```rust
pub mod config;
pub mod memory;
pub mod retrieval;
pub mod policy;
pub mod store;
pub mod query;
```

**Responsibilities:**

- `store.rs` — Postgres migration runner (using `sqlx::migrate!`), reading CSL-generated SQL from a `migrations/` directory.
- `memory.rs` — writes chunks to vector store + typed rows to Postgres in a single transaction when possible (saga pattern when not).
- `retrieval.rs` — hybrid search: parallel BM25 (tantivy in-proc or Postgres full-text) + vector (via `VectorStore` trait) + reranker.
- `query.rs` — takes a `ContextQuery { query: String, caller: Caller, budget: TokenBudget, filters: Filters }` and produces a `ContextResponse { chunks, receipt }`.
- `policy.rs` — loads compiled WASM predicates (stub: always-allow for Phase 4; real impl in Phase 6), applies to candidate chunks.

Add `sqlx` migration directory `crates/signet-engine/migrations/20260101000000_init.sql`:

```sql
CREATE TABLE IF NOT EXISTS signet_schemas (
    namespace   text not null,
    name        text not null,
    version     integer not null,
    definition  jsonb not null,
    created_at  timestamptz not null default now(),
    primary key (namespace, name, version)
);

CREATE TABLE IF NOT EXISTS signet_connectors (
    id          ulid primary key,
    name        text not null,
    config      jsonb not null,
    enabled     boolean not null default true,
    last_sync_at timestamptz,
    created_at  timestamptz not null default now()
);

CREATE TABLE IF NOT EXISTS signet_receipts (
    id          ulid primary key,
    caller      text not null,
    query       text not null,
    sources     jsonb not null,
    policies    jsonb not null,
    answer_hash bytea,
    created_at  timestamptz not null default now()
);
```

(Note: if Postgres does not have a native `ulid` type, use `bytea` or `uuid` and document the choice. The `ulid` extension via `pg_idkit` is one option; absent that, store as `bytea`.)

Run `cargo check -p signet-engine`.

Commit: `git commit -am "feat(engine): memory and retrieval skeletons"`.

---

## Phase 5 — Gateway (REST + MCP)

**`crates/signet-gateway/src/lib.rs`**
```rust
pub mod server;
pub mod rest;
pub mod mcp;
pub mod auth;
pub mod config;
```

**Gateway duties:**

- `server.rs` — `axum::Router` composition, tower middleware (trace, cors, timeout, compression).
- `rest.rs` — endpoints:
  - `POST /v1/query` → calls engine `query::resolve`.
  - `GET /v1/schemas` / `GET /v1/schemas/:name`
  - `POST /v1/connectors` / `GET /v1/connectors` / `POST /v1/connectors/:id/sync`
  - `GET /v1/receipts/:id`
  - `GET /healthz`
- `mcp.rs` — MCP server implementation:
  - Stateless HTTP transport (SEP-current streamable HTTP).
  - `initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`.
  - Auto-registered tools: for each schema X → `search_X(query, top_k?, filters?)`, `get_X(id)`.
  - OAuth 2.1 metadata endpoint stub at `/.well-known/oauth-authorization-server` (full OAuth in Phase 7).
- `auth.rs` — JWT validation middleware (short-term: accept any signed JWT from a configured JWKS URL; OIDC/OAuth flows come in Phase 7).

Add an integration test:
`crates/signet-gateway/tests/health.rs`:
```rust
#[tokio::test]
async fn healthz_returns_200() {
    let app = signet_gateway::server::build_app(signet_gateway::config::Config::test()).await.unwrap();
    let response = axum::serve::call(&app, http::Request::builder()
        .uri("/healthz").body(axum::body::Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), 200);
}
```

**Verify**: `cargo test -p signet-gateway` passes.

Commit: `git commit -am "feat(gateway): rest and mcp surfaces"`.

---

## Phase 6 — Reference Connectors (3 minimum for the demo)

Create `connectors/github`, `connectors/slack`, `connectors/local-files`.

Each is a separate crate that depends on `signet-connector-sdk`. Each implements the `Connector` trait and exposes a `register()` function that the ingest runtime uses to load it.

**Demo target: `connectors/local-files`** (the simplest, required for the end-to-end acceptance test):
- `new(root: PathBuf)` constructor.
- `list()` walks the directory and yields one `Resource` per file with `body = fs::read_to_string(path)` for text formats, a title from the filename, and a single `permission { principal: "*", action: "read" }`.
- `fetch(id)` — id is the absolute path; reread the file.
- `subscribe()` — use `notify` crate to watch for changes; yield `ChangeEvent::Upsert` on change and `::Delete` on removal.

**`connectors/github`**: OAuth-device-flow auth, `list()` iterates repos the token can access, yields issues, PRs, and README content. Can be stubbed with `todo!()` for Phase 6 if it extends scope — the local-files connector alone satisfies the acceptance test.

**`connectors/slack`**: bot token auth, `list()` yields messages from configured channels.

Commit: `git commit -am "feat(connectors): local-files, github, slack connectors"`.

---

## Phase 7 — SDKs and Console

### TypeScript SDK

`sdks/typescript/` — a small axios/fetch wrapper generated (partly) from the gateway's OpenAPI spec. Minimum surface:
- `new SignetClient({ baseUrl, token })`
- `.query(q, opts)` → `Promise<ContextResponse>`
- `.schemas.list()` / `.schemas.get(name)`
- `.connectors.list()` / `.connectors.sync(id)`

Publish config under `@signet/sdk` with `exports` for ESM + CJS.

### Python SDK

`sdks/python/signet/` — Pydantic v2 models + an `httpx` client. Mirrors the TS surface.

### Console (SvelteKit)

`console/` — init with `pnpm create svelte@latest .`. Choose:
- Skeleton project
- TypeScript
- Prettier + ESLint + Playwright

Install: `tailwindcss`, `@tanstack/svelte-query`, `shadcn-svelte`.

Routes (all placeholders this phase):
- `/` — dashboard
- `/schemas` — list + detail
- `/connectors` — list + CRUD
- `/query` — the "playground" page: query bar + JSON result viewer
- `/receipts/[id]` — receipt viewer
- `/settings`

The console calls the gateway's REST endpoints. No business logic lives in the SvelteKit app beyond rendering.

**Verify**: `pnpm -C console build` succeeds.

Commit: `git commit -am "feat(sdks+console): typescript/python sdks and sveltekit console skeleton"`.

---

## Phase 8 — Dev Stack (Docker Compose) and CLI `signet dev`

**`deploy/docker/compose.dev.yaml`**
```yaml
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_PASSWORD: signet
      POSTGRES_USER: signet
      POSTGRES_DB: signet
    ports: ["5432:5432"]
    volumes: [signet_pg:/var/lib/postgresql/data]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U cfg"]
      interval: 2s

  qdrant:
    image: qdrant/qdrant:latest
    ports: ["6333:6333", "6334:6334"]
    volumes: [signet_qdrant:/qdrant/storage]
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:6333/readyz || exit 1"]
      interval: 2s

  redis:
    image: redis:7-alpine
    ports: ["6379:6379"]

  gateway:
    build:
      context: ../..
      dockerfile: deploy/docker/Dockerfile.gateway
    ports: ["7070:7070"]
    environment:
      SIGNET_DATABASE_URL: postgres://signet:signet@postgres:5432/signet
      SIGNET_QDRANT_URL: http://qdrant:6334
      SIGNET_REDIS_URL: redis://redis:6379
    depends_on:
      postgres: { condition: service_healthy }
      qdrant:   { condition: service_healthy }
      redis:    { condition: service_started }

volumes:
  signet_pg:
  signet_qdrant:
```

**`deploy/docker/Dockerfile.gateway`**
```dockerfile
FROM rust:1.83-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release --bin signet-gateway

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /build/target/release/signet-gateway /usr/local/bin/signet-gateway
EXPOSE 7070
ENTRYPOINT ["/usr/local/bin/signet-gateway"]
```

Add a binary target to `signet-gateway`:
```toml
[[bin]]
name = "signet-gateway"
path = "src/bin/server.rs"
```

Implement `signet dev` in `crates/signet-cli/src/cmd_dev.rs` to:
1. Locate the compose file (relative to the crate root or via `$SIGNET_HOME`).
2. `docker compose -f <path> up -d --build`.
3. Poll `http://localhost:7070/healthz` until 200 or 60s timeout.
4. Print a ready banner with URLs for gateway (`:7070`), console (`:7071` — the SvelteKit dev server started separately), Qdrant UI (`:6333/dashboard`), Postgres (`localhost:5432`).

**Verify**: from a clean machine (no running services), `signet dev` returns within 60 seconds with the ready banner, and `curl http://localhost:7070/healthz` returns 200.

Commit: `git commit -am "feat(devx): docker compose stack and signet dev command"`.

---

## Phase 9 — CI (GitHub Actions)

**`.github/workflows/ci.yml`**
```yaml
name: CI
on:
  push:  { branches: [main] }
  pull_request:
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.83
        with: { components: rustfmt, clippy }
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace --locked

  rust-audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.83
      - uses: EmbarkStudios/cargo-deny-action@v2

  node:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: pnpm }
      - run: pnpm install --frozen-lockfile
      - run: pnpm -C console lint
      - run: pnpm -C console check
      - run: pnpm -C console build
      - run: pnpm -C sdks/typescript build
      - run: pnpm -C sdks/typescript test

  integration:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16-alpine
        env:
          POSTGRES_USER: signet
          POSTGRES_PASSWORD: signet
          POSTGRES_DB: signet
        ports: ["5432:5432"]
        options: >-
          --health-cmd "pg_isready -U cfg"
          --health-interval 2s --health-timeout 5s --health-retries 10
      qdrant:
        image: qdrant/qdrant:latest
        ports: ["6333:6333", "6334:6334"]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.83
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --test e2e --locked
        env:
          SIGNET_DATABASE_URL: postgres://signet:signet@localhost:5432/signet
          SIGNET_QDRANT_URL:   http://localhost:6334
```

**`.github/workflows/security.yml`** — cargo-audit nightly.
**`.github/workflows/release.yml`** — release-please on tag push.

Commit: `git commit -am "ci: main pipeline, audit, integration"`.

---

## Phase 10 — pnpm Root + Workspace Config

**`pnpm-workspace.yaml`**
```yaml
packages:
  - "console"
  - "sdks/typescript"
```

**`package.json`** (root)
```json
{
  "name": "signet-js-root",
  "private": true,
  "packageManager": "pnpm@9.15.0",
  "scripts": {
    "lint": "pnpm -r lint",
    "build": "pnpm -r build",
    "test": "pnpm -r test"
  },
  "engines": { "node": ">=20" }
}
```

Run `pnpm install` at the root. Commit.

---

## Phase 11 — First Schema + First Demo

Create `examples/engineering-context/schemas/doc.csl`:
```csl
schema Doc {
    id:         Ulid    @primary
    path:       String  @searchable @indexed
    title:      String  @searchable
    body:       Text    @chunked
    updated_at: Instant

    context {
        chunking        = semantic(max_tokens = 512, overlap = 64)
        embedder        = "stub"
        vector_dims     = 64
        default_top_k   = 8
        freshness_decay = exponential(half_life = 30d)
    }
}
```

Create `examples/engineering-context/README.md` with steps:
```bash
signet dev
signet schema apply schemas/doc.csl
signet connector add local-files --root ./sample-docs
signet connector sync local-files
signet query "what does the setup guide say about postgres?"
```

Populate `sample-docs/` with a handful of markdown files that include "postgres" keywords.

**Verify**: the query returns hits with scores > 0 and a receipt ID. The receipt, fetched at `/v1/receipts/:id`, shows the source files that were retrieved.

Commit: `git commit -am "feat(examples): engineering-context end-to-end demo"`.

---

## Phase 12 — Acceptance Test

This is the gate for declaring the scaffold complete. It runs as part of CI's `integration` job.

`crates/signet-gateway/tests/e2e.rs`:
```rust
//! End-to-end acceptance test.
//! Preconditions:
//!   * Postgres and Qdrant reachable via env.
//!   * stub embedder compiled in.
//! Asserts:
//!   1. schema apply creates the collection and MCP tool
//!   2. sync ingests sample docs
//!   3. query returns >=1 hit with a receipt
//!   4. MCP /tools/list includes `search_doc`
//!   5. MCP /tools/call search_doc returns at least 1 result
//!   6. Receipt includes provenance back to the source file

#[tokio::test(flavor = "multi_thread")]
async fn scaffold_end_to_end() -> anyhow::Result<()> {
    let env = signet_test::Env::spin_up().await?;
    env.apply_schema("examples/engineering-context/schemas/doc.csl").await?;
    env.add_local_files_connector("examples/engineering-context/sample-docs").await?;
    env.sync_connector("local-files").await?;
    let r = env.query("postgres setup").await?;
    assert!(!r.chunks.is_empty());
    assert!(r.receipt.id.to_string().len() > 0);

    let tools = env.mcp_tools_list().await?;
    assert!(tools.iter().any(|t| t.name == "search_doc"));

    let mcp_result = env.mcp_call("search_doc", serde_json::json!({"query": "postgres"})).await?;
    assert!(mcp_result["results"].as_array().is_some_and(|a| !a.is_empty()));

    let receipt = env.get_receipt(&r.receipt.id).await?;
    assert!(!receipt.sources.is_empty());
    Ok(())
}
```

Run `cargo test --test e2e`. If green, the scaffold is complete.

Commit: `git commit -am "test: end-to-end acceptance"`.

Tag: `git tag v0.1.0-scaffold && git push --tags`.

---

## Completion Checklist

- [ ] `cargo build --workspace` succeeds.
- [ ] `cargo test --workspace` passes.
- [ ] `pnpm -C console build` succeeds.
- [ ] `pnpm -C sdks/typescript build` succeeds.
- [ ] `signet version` prints the expected version.
- [ ] `signet dev` starts the stack within 60 seconds.
- [ ] `signet schema apply`, `connector add`, `connector sync`, `query` all succeed against the running stack.
- [ ] `/v1/query` returns a receipt ID for every call.
- [ ] `/mcp/tools/list` exposes `search_doc`.
- [ ] CI is green on `main`.
- [ ] `LICENSE`, `NOTICE`, `TRADEMARKS.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CONTRIBUTING.md`, `GOVERNANCE.md`, `CHANGELOG.md` all present at repo root.

---

## What's Explicitly Out of Scope for the Scaffold

These come in post-scaffold milestones and should not block the v0.1.0-scaffold tag:

- Real OAuth 2.1 / OIDC flows (Phase 7 of the product roadmap).
- Real embedders (OpenAI, Voyage, Cohere) — stub embedder is sufficient for the scaffold.
- Vector store backends beyond Qdrant.
- Cluster mode (lives in the private enterprise repo).
- Premium connectors (SAP, Oracle, etc.).
- Policy WASM compilation from CSL — the parser accepts policy blocks but codegen is stubbed.
- Web console beyond placeholder routes.
- Python SDK beyond a thin REST wrapper.
- FedRAMP / SOC 2 infrastructure.

Each of these is called out in the main roadmap and tracked as a separate milestone.

---

## Notes for the Agent Executor

1. **Read the full brief before running any phase.** If any phase seems to contradict an earlier decision, stop and report.
2. **Do not invent APIs.** If a crate's trait method signature is not in this brief, use the simplest plausible form and mark with `// FIXME: confirm signature`.
3. **Use `todo!()` liberally.** Stubs are encouraged; the acceptance criterion is that the scaffold builds, not that every feature is implemented.
4. **Commit after each phase** with the commit message shown. Do not squash commits.
5. **If a specified dependency version is unavailable at execution time**, choose the nearest minor release and note it in the commit message.
6. **Stop on the first unrecoverable error.** Do not hallucinate fixes — report and wait.

*End of scaffolding brief.*
