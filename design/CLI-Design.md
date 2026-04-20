# CLI — Design Notes

**Companion to**: `crates/signet-cli/` in the drop-in, plus the updated `crates/signet-gateway/src/{rest,server,bin/server}.rs`.
**Status**: Unverified skeleton. 1,539 LOC for the CLI, ~300 LOC of gateway REST handlers, plus a small patch to `signet-csl` that emits schema metadata for the CLI to forward.

---

## 1. What the CLI actually is

Three things, in order of weight:

1. **A thin HTTP client** over the gateway's REST surface. `signet schema list`, `signet connector sync`, `signet query`, `signet receipt` — all of them just hit an endpoint and render the response.
2. **A local CSL compiler driver.** `signet compile` and `signet schema validate` run `signet_csl::compile()` in-process without touching the gateway. `signet schema apply` is the hybrid — compile locally, then POST the results.
3. **A Docker Compose wrapper.** `signet dev` shells out to `docker compose up -d` against a well-known file path, then polls `/healthz` until ready.

It is deliberately not a general-purpose shell for every engine operation. Anything that needs the engine's internals (constructing receipts from scratch, manipulating the vector store directly) belongs in other tooling.

---

## 2. Architecture

```
  cfg <subcommand> ────▶ cli::Cli::parse
                              │
                              ▼
                        commands::Context (gateway url, user, project root, format)
                              │
                              ▼
                       commands::<sub>::run
                              │
                       ┌──────┴──────┐
                       │             │
                       ▼             ▼
                 client::Client   signet_csl::compile  (+ filesystem I/O)
                 (HTTP client)
                       │
                       ▼
                 gateway REST surface
```

Every command takes a single `Context` struct plus its typed args from `cli.rs`. No global state, no implicit config magic beyond environment variable defaults in `clap`.

### File layout

| File | Purpose |
|---|---|
| `src/main.rs` | Clap dispatch + tracing setup |
| `src/cli.rs` | Clap types for every subcommand |
| `src/client.rs` | HTTP client with envelope unwrap |
| `src/project.rs` | `cfg.toml` discovery + parsing + scaffolding |
| `src/output.rs` | JSON-or-table formatter (hand-rolled, no `comfy-table`) |
| `src/commands.rs` | Shared `Context` struct + module declarations |
| `src/commands/dev.rs` | Docker Compose wrapper |
| `src/commands/init.rs` | Project scaffolder |
| `src/commands/compile.rs` | Compile every `.csl` under `schemas/` |
| `src/commands/schema.rs` | apply / list / get / validate |
| `src/commands/connector.rs` | add / list / sync |
| `src/commands/query.rs` | Search + receipt id |
| `src/commands/receipt.rs` | Fetch one receipt |
| `src/commands/version.rs` | Build + health info |

---

## 3. The end-to-end flow the CLI enables

With this drop-in, the scaffolding brief's Phase 11 demo becomes a real CLI session:

```bash
# 1. Scaffold (creates cfg.toml, schemas/doc.csl, sample-docs/)
signet init

# 2. Boot the stack
signet dev              # docker compose up -d + poll /healthz

# 3. Register the schema (compile locally + POST to gateway)
signet schema apply schemas/doc.csl
signet schema list

# 4. Bind a connector and sync
signet connector add local-files --schema examples.Doc --root ./sample-docs
signet connector list
signet connector sync local-files/examples.Doc

# 5. Query
signet query "getting started" --schema examples.Doc
signet receipt <receipt_id>
```

Every step is under 100 ms of CLI time plus whatever the gateway takes (roughly 10 ms for schema/connector ops, 100–300 ms for a search against the stub embedder).

---

## 4. Schema apply — the non-trivial command

`signet schema apply <path>` is the only command that splits work between the CLI and the gateway:

1. CLI reads the `.csl` file from disk.
2. CLI calls `signet_csl::compile(src, CompileTargets::all())`.
3. For each `schema_meta` in the returned `CompileOutput.schemas_meta`:
   - `POST /v1/schemas` with `{ "meta": <schema_meta> }` — the gateway hydrates its registry.
   - `POST /v1/schemas/<qualified>/ddl` with `{ "ddl": <sql> }` — the gateway runs the DDL through `Store::apply_schema_ddl` (transactional `sqlx` execution).

Two round trips, not one. The split matters because the DDL is persisted in Postgres and survives a gateway restart, while the in-memory registry does not — the next restart will re-hydrate from `signet_schemas` if we wire that up, but today the operator re-runs `signet schema apply` on restart.

### Why didn't I just have the gateway compile?

Two reasons. First, compilation is the single most expensive CLI operation, and it's doing work the operator wants feedback on *locally* — syntax errors should surface before anything hits the network. Second, the compiler is a big chunk of code (the winnow parser, the type checker, the codegen modules). Embedding it in the gateway would grow the production binary without real benefit; embedding it in the CLI is a one-time 300 ms compile-time cost that's already been paid.

### The signet-csl patch

To make `schema apply` work, I patched `signet-csl` to emit a `schemas_meta: Vec<Value>` field on `CompileOutput`. Each entry is a JSON document matching `signet_engine::schema_registry::SchemaMeta` — namespace, name, version, primary key, fields with decorator flags (`primary`, `indexed`, `searchable`, `chunked`, `facet`, `unique`, `optional`, plus `boost` and `pii` values), relations, context block, collection name (`<table>_v<version>`), table name (snake_cased schema name), and optional schema-level hybrid alpha.

The emitter is ~150 lines in `codegen/mod.rs`. It walks the `TypedFile`'s schema list and produces one JSON object per schema. Decorator inference is first-match; type rendering is Debug-style for primitives plus explicit shapes for `Ref<T>`, `List<T>`, `Map<K, V>`, `Vector<N>`, and `T?`.

**Caveat:** the emitter makes strong assumptions about the `signet-csl` AST shape — specifically `FieldDecl.decorators`, `Decorator::is()`, `Arg.name`, `Expr::Call`, `Literal::Duration`, and `rel.kind.as_str()`. If any of those differ from what my AST module declares, the patch will fail to compile. The fix is local and mechanical — adjust the field accesses to match the actual AST.

---

## 5. Auth

For v0.1 the CLI identifies itself through three headers:

- `X-Cfg-User` — caller id. Defaults to `$USER` or `"anon"`.
- `X-Cfg-Tenant` — tenant slug. Empty means default tenant.
- `X-Cfg-Roles` — comma-separated role list.

The gateway's `CallerExt` extractor picks these up and constructs a `signet_engine::api::Caller` that flows through the engine's policy layer.

This is **not** production auth. The CSL spec and the engine design doc both plan JWT validation middleware ahead of the extractor — that's where production deployments will land. The header-based path exists so operators can exercise the full end-to-end system from the CLI today without standing up an identity provider.

---

## 6. Output format

Two modes selected by `--json`:

- **Human** (default). For array-of-objects values, renders a padded ASCII table: union-of-keys columns, truncated cells at 80 chars, column-aligned. For scalars, prints them verbatim. For objects, prints `key: value` lines aligned on the longest key.
- **JSON**. Pretty-prints the unwrapped `data` field of the gateway envelope. Scripts pipe through `jq`.

The table renderer is hand-rolled — 80 lines in `output.rs`. It handles what matters for this CLI (small-to-medium rowsets, mixed-type columns) and avoids the dependency cost of `comfy-table` or `tabled`.

---

## 7. Gateway-side changes to support the CLI

Patches required to the gateway (included in this drop-in):

1. **`AppState`** gains `engine: Arc<Engine>`, `ingest: Arc<IngestRuntime>`, `connector_factory: ConnectorFactory`. The existing `engine_facade: Arc<dyn EngineFacade>` is still there for MCP dispatch; it's a coerced clone of the same `Arc<Engine>`.

2. **`build_app` signature** takes `engine: Arc<Engine>` and `connector_factory: ConnectorFactory` now. The binary constructs both at startup; the factory knows about `local-files` and rejects unknown kinds with a clear error.

3. **Eleven real REST handlers** replace the old `NOT_IMPLEMENTED` stubs. All use the envelope shape `{ data, error }`. `EngineError` variants map to `(StatusCode, code)` pairs in `engine_error_response`.

4. **`Engine::store_handle()`** is a new one-line public getter on the engine so REST handlers can run `apply_schema_ddl`. No deeper surface exposed.

5. **`signet-csl` emits `schemas_meta`** on `CompileOutput`. See §4 above.

---

## 8. Known gaps — priority order

1. **Compile-error rendering.** `signet schema validate` currently prints `CslError::Display`. The CSL spec calls for `miette` rendering with spans, underlines, and help text. Adding `miette` as a CLI dep and a one-liner `Report::new(err).with_source_code(src)` gives the full experience.

2. **Real auth.** Header-based identity is a dev convenience. Wire in JWT validation in a Tower middleware layer ahead of `CallerExt`. Operators get an `OAUTH_TOKEN` or `SIGNET_TOKEN` env var; the CLI sends it as a bearer. Target: next sprint.

3. **Receipt retrieval latency.** `signet receipt <id>` hits `/v1/receipts/<id>`, which does a single `SELECT` against `signet_receipts`. Fast, but there's no caching. Not a problem yet.

4. **No interactive confirmation.** `signet schema apply` on an existing schema silently overwrites the in-memory registry entry. A real production flow should prompt unless `--yes` is passed.

5. **`cfg watch` is missing.** The local-files connector already supports `subscribe`. The CLI should expose `signet connector watch <id>` that spawns a long-running session tailing change events. ~50 LOC.

6. **Background connector scheduling.** `signet connector add` registers a binding with `interval = None`; the gateway's `IngestRuntime::start_background` is never called. For operational deployments we want a `--interval 5m` flag plus a `signet connector schedule` command.

7. **`signet dev --tail-logs`.** After `docker compose up -d`, many operators immediately run `docker compose logs -f`. A `--tail-logs` flag should just do that inline.

8. **Shell completions.** `clap_complete` generates completion scripts for bash/zsh/fish/powershell. `cfg completions <shell>` is one match arm.

9. **Non-UTF-8 filesystem handling.** `LocalFilesConnector` uses `path.display().to_string()` for resource ids. On Windows with non-UTF-8 paths, this can produce inconsistent ids. Not a v0.1 concern.

10. **Rate-limit the `healthz` poll in `signet dev`.** Currently 500 ms. Should back off exponentially to avoid log spam when the gateway takes 30+ seconds to come up on a fresh docker build.

---

## 9. Verification checklist

Things most likely to need adjustment against a live compiler:

1. **signet-csl AST field names.** My `emit_schemas_meta` in `codegen/mod.rs` assumes specific field layouts on `SchemaDecl`, `FieldDecl`, `Decorator`, `Expr`, and the literal variants (`Duration`, `Float`, `Integer`, `String`). If any of these names differ, the patch needs one-line fixes. The `Decorator::is(name)` method call also needs to exist.

2. **`rel.kind.as_str()`.** The CSL `RelationKind` enum needs a `as_str` method that returns `"one"` or `"many"`. If it doesn't, add one or inline the match.

3. **Axum extractor trait.** `FromRequestParts` in axum 0.8 uses `async_trait` macros; the `impl<S: Sync> FromRequestParts<S> for CallerExt` block needs to align with whatever axum 0.8 provides. If the compiler complains, check the axum examples.

4. **`reqwest::Client::builder().user_agent()`.** Stable API; shouldn't shift. If it does, drop the user-agent line.

5. **`tokio::process::Command::status()` await.** Returns `io::Result<ExitStatus>`. Stable.

6. **clap `env = "SIGNET_GATEWAY_URL"` + `default_value = "..."`.** Stable since clap 4.0.

7. **`toml` 0.8 deserializer.** Works through `toml::from_str`. Stable.

If `cargo check -p signet-cli` passes with the CLI alone — which doesn't depend on `signet-engine`, `signet-gateway`, or the connector SDK — the bulk of the code is valid. The gateway-side patches are the higher-risk surface.

---

## 10. Running the acceptance test

The scaffolding brief's Phase 12 e2e test can now be written against the real CLI:

```bash
#!/usr/bin/env bash
set -euo pipefail

signet init
signet dev
signet schema apply schemas/doc.csl
signet connector add local-files --schema examples.Doc --root ./sample-docs
signet connector sync local-files/examples.Doc
result=$(signet query "getting started" --schema examples.Doc --json)
echo "$result" | jq -e '.results | length > 0'
receipt=$(echo "$result" | jq -r .receipt_id)
signet receipt "$receipt" --json | jq -e '.sources | length > 0'
```

Every line hits a real HTTP endpoint backed by real engine code. When this script exits 0 on CI, v0.1 is feature-complete for its acceptance criterion.

*End of CLI design notes.*
