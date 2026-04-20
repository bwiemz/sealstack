# ContextForge

**The open, developer-first context operating system for enterprises.**

MCP-native · Self-hostable · Typed context schemas

---

## Quick start

```bash
# Clone and bring up the full local stack
git clone https://github.com/bwiemz/contextforge
cd contextforge
cargo install --path crates/cfg-cli
cfg dev
```

`cfg dev` starts Postgres, Qdrant, Redis, and the gateway on your laptop in
under 60 seconds. Point it at some local markdown and you have a working
context layer the same afternoon.

## What is this

ContextForge unifies ingestion, retrieval, memory, policy, and tool-use under
a single typed context model, and exposes every internal system as a governed
MCP server. See [`design/ContextForge-Plan.md`](./design/ContextForge-Plan.md) for the full
product plan and [`design/CSL-Specification.md`](./design/CSL-Specification.md) for the
Context Schema Language spec.

## Repository layout

- `crates/` — Rust workspace (engine, gateway, CSL compiler, CLI, SDK traits)
- `connectors/` — first-party connectors (GitHub, Slack, local files, …)
- `sdks/` — TypeScript and Python client SDKs
- `console/` — SvelteKit admin console
- `deploy/` — Docker Compose, Helm, Terraform
- `examples/` — runnable demos
- `docs/` — documentation site

## License

Apache-2.0. See [`LICENSE`](./LICENSE), [`NOTICE`](./NOTICE), and
[`TRADEMARKS.md`](./TRADEMARKS.md).
