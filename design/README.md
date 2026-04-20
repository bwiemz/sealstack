# Signet design docs

Internal specifications, design briefs, and planning documents for the
Signet platform. These are the authoritative "why" for every decision
in the codebase.

## Specifications

- [`CSL-Specification.md`](./CSL-Specification.md) — the Context Schema Language grammar and semantics.
- [`CSL-Parser-Design.md`](./CSL-Parser-Design.md) — how `signet-csl` parses CSL.
- [`MCP-Generator-Design.md`](./MCP-Generator-Design.md) — how CSL schemas compile to MCP tools.

## Component designs

- [`Engine-Design.md`](./Engine-Design.md) — retrieval, ingestion, receipts, policy.
- [`Backends-Design.md`](./Backends-Design.md) — Postgres + Qdrant + embedder/reranker abstractions.
- [`Ingestion-Design.md`](./Ingestion-Design.md) — connector runtime, resource shape, tenant scoping.
- [`CLI-Design.md`](./CLI-Design.md) — `cfg` CLI surface.
- [`Console-Design.md`](./Console-Design.md) — SvelteKit admin UI aesthetic + IA.

## Planning + strategy

- [`Signet-Plan.md`](./Signet-Plan.md) — the full product plan and phase roadmap.
- [`Signet-Scaffolding-Brief.md`](./Signet-Scaffolding-Brief.md) — repo scaffolding contract for v0.1.
- [`Signet-vs-Glean-BattleCard.md`](./Signet-vs-Glean-BattleCard.md) — positioning vs Glean.

User-facing documentation (getting-started, tutorials, API references) lives
in [`../docs/`](../docs/) and is published to the Starlight site.
