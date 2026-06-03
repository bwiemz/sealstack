# SealStack Agent Guide

## Read First

- [README.md](README.md) for the repo overview and local quick start.
- [CONTRIBUTING.md](CONTRIBUTING.md) for required validation commands and commit message format.
- [design/SealStack-Plan.md](design/SealStack-Plan.md) for the product model and layer map.
- [docs/src/content/docs/architecture.md](docs/src/content/docs/architecture.md) for the current high-level architecture.
- [design/Engine-Design.md](design/Engine-Design.md) as background notes for engine or gateway work; confirm behavior in the owning crate before changing code.

When a design note disagrees with current code, tests, or docs, trust the live implementation and treat the design note as background context only.

Use the area-specific docs instead of restating them in code or instructions:

- CSL: [design/CSL-Specification.md](design/CSL-Specification.md), [design/CSL-Parser-Design.md](design/CSL-Parser-Design.md), [design/MCP-Generator-Design.md](design/MCP-Generator-Design.md)
- Connectors and ingestion: [design/Ingestion-Design.md](design/Ingestion-Design.md), [connectors/README.md](connectors/README.md), [connectors/local-files](connectors/local-files)
- CLI: [design/CLI-Design.md](design/CLI-Design.md) as background notes; prefer the current `crates/sealstack-cli` commands and gateway tests when behavior differs
- Console: [design/Console-Design.md](design/Console-Design.md), [console/README.md](console/README.md)
- Docs site and public API docs: [docs/README.md](docs/README.md), [docs/src/content/docs/architecture.md](docs/src/content/docs/architecture.md), [docs/src/content/docs/api/rest.md](docs/src/content/docs/api/rest.md), [docs/src/content/docs/api/mcp.md](docs/src/content/docs/api/mcp.md)

## Workspace Map

- `crates/` contains the core Rust workspace: engine, dedicated ingest runtime, gateway, CLI, CSL compiler, policy, vector store, embedders, receipts, and shared types.
- `connectors/` contains first-party connector crates. Treat `connectors/local-files` as the reference implementation for connector shape and conventions.
- `console/` is the SvelteKit admin console.
- `docs/` is the Astro documentation site.
- `sdks/` contains the maintained TypeScript and Python SDKs plus a scaffold-only Go SDK directory.
- `deploy/`, `helm/`, and `terraform/` contain deployment assets, not runtime source.
- `examples/engineering-context/` is the best end-to-end sample for schemas plus ingestion.
- `target/` is generated build output. Do not edit it.

## Repo-Specific Rules

- Use the pinned Rust toolchain from [rust-toolchain.toml](rust-toolchain.toml). The workspace is edition 2024 and uses resolver `3`.
- `unsafe` is forbidden workspace-wide. The only expected exception area is the policy runtime crate.
- Keep Rust dependencies and lints inherited from the workspace when adding or editing crates.
- Keep new Rust crates and internal packages aligned with the existing `sealstack-*` naming pattern.
- Prefer narrow, owning-crate edits over cross-workspace churn. Start from the crate or package that directly owns the behavior.
- Use async-friendly I/O on hot paths. Avoid introducing blocking filesystem or network calls into async request handling.
- For CSL codegen or parser work, follow the existing `insta` snapshot-test pattern instead of inventing a new verification style.
- Root `pnpm` scripts only cover the JavaScript workspace packages. They do not validate the Rust crates or the Python SDK.
- Follow Conventional Commits for commit messages: `type(scope): subject`.

## Commands

### Rust workspace

```bash
cargo check --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

For focused work, prefer crate-scoped validation first:

```bash
cargo test -p <crate>
cargo clippy -p <crate> --all-targets -- -D warnings
```

### Console

```bash
pnpm -C console dev
pnpm -C console check
pnpm -C console lint
pnpm -C console build
```

### Docs

`docs/` is not part of the root pnpm workspace and uses its own npm lockfile, so install and run it as a separate npm project.

```bash
cd docs
npm install
npm run dev
npm run check
npm run build
```

### TypeScript SDK

```bash
pnpm -C sdks/typescript lint
pnpm -C sdks/typescript test
pnpm -C sdks/typescript build
```

### Python SDK

```bash
pip install -e "sdks/python[dev]"
ruff check sdks/python
basedpyright sdks/python
```

The Python SDK currently has lint and type-check coverage in-repo, but no committed test suite yet.
`basedpyright` is expected to be available in the environment; it is not installed by the SDK's `dev` extra.

### Local stack

Run these from the repository root with Docker available. If you need to start from elsewhere, pass `--compose-file` explicitly or set `SEALSTACK_HOME`.

```bash
cargo install --path crates/sealstack-cli
sealstack dev
```

### JS workspace aggregate

```bash
pnpm lint
pnpm test
pnpm build
```

## Change Routing

- `crates/sealstack-csl` owns CSL parsing, typing, and code generation.
- `crates/sealstack-common` owns shared identifiers, errors, and cross-crate types.
- `crates/sealstack-engine` owns search, schema state, and core policy-aware engine integration.
- `crates/sealstack-embedders` owns embedder abstractions and vendor-backed embedding implementations.
- `crates/sealstack-ingest` owns connector execution, background sync flow, and feeding resources into the engine.
- `crates/sealstack-gateway` owns the REST and MCP server surface.
- `crates/sealstack-cli` owns local dev workflows and command-line user flows.
- `crates/sealstack-connector-sdk` plus `connectors/local-files` define the connector contract and the clearest implementation pattern.
- `crates/sealstack-receipts` owns receipt data structures and receipt-focused storage primitives.
- `crates/sealstack-vectorstore` owns vector-store abstractions and backend implementations such as in-memory and Qdrant.
- `crates/sealstack-policy-ir` and `crates/sealstack-policy-runtime` own policy IR invariants and runtime execution. Policy bundle code generation lives in `crates/sealstack-csl`.
- `console/` and `docs/` are separate frontend projects with their own checks and build pipelines.

## Before Finishing

- Run the narrowest relevant checks for the area you changed before widening to workspace-wide validation.
- If you touch `crates/sealstack-policy-runtime` or policy runtime inputs, make sure the `wasm32-unknown-unknown` target is installed, run `./scripts/rebuild-policy-runtime.sh`, and commit the updated `crates/sealstack-csl/assets/policy_runtime.wasm` asset.
- If you change public API surfaces, connector contracts, or CSL semantics, update the closest doc or design reference rather than adding free-standing explanation elsewhere.
- If snapshot tests change, inspect the diff intentionally before accepting it.
- Keep instructions and docs linked, concise, and current. Do not duplicate long architecture prose into new customization files.
