---
title: Architecture overview
description: The major components of a running Signet deployment and how they interact.
---

Signet is intentionally small. The entire runtime is five moving parts:

```text
┌──────────────────┐      ┌──────────────────────┐
│  MCP client      │─────▶│  Gateway             │
│  (Claude, etc.)  │      │  (REST + MCP + OAuth)│
└──────────────────┘      └──────────┬───────────┘
┌──────────────────┐                 │
│  Console UI      │─────────────────┤
└──────────────────┘                 │
┌──────────────────┐                 ▼
│  CLI             │          ┌────────────┐
└──────────────────┘          │  Engine    │
                              │  (Rust)    │
                              └──┬──────┬──┘
                     ┌───────────┘      └───────────┐
                     ▼                              ▼
               ┌───────────┐                ┌───────────────┐
               │ Postgres  │                │  Qdrant       │
               │  (rows,   │                │  (vectors +   │
               │  receipts │                │   BM25 filter)│
               │  FTS)     │                │               │
               └───────────┘                └───────────────┘
```

## Crates

- **signet-csl** — parses and type-checks CSL, emits DDL, MCP tool descriptors, and typed SDK clients.
- **signet-engine** — retrieval, ingest, receipts, policy. Every business behavior lives here.
- **signet-gateway** — HTTP surface. REST for humans and CLIs, MCP JSON-RPC streamable-HTTP for agents, OAuth 2.1 discovery + bearer validation.
- **signet-ingest** — runtime that drives connectors on a schedule and feeds resources into the engine's ingest path.
- **signet-connector-sdk + connectors/\*** — trait definition and bundled connector implementations (local-files, GitHub, Slack).

## Data plane

Schemas and connector bindings are durable: they live in Postgres
(`signet_schemas`, `signet_connectors`) and are rehydrated on every gateway boot. A
restart loses nothing.

Every per-schema table gets a `tenant` column automatically. Retrieval queries
scope by `coalesce(tenant,'') = $caller_tenant`, enforcing multi-tenant
isolation at the SQL boundary.

## Control plane

The gateway is stateless at the request level — sessions are the only
in-memory state. All schemas, connector bindings, and receipts persist to
Postgres; vector state persists to Qdrant. Horizontal scale is straightforward:
multiple gateway replicas behind a load balancer, sharing the same Postgres +
Qdrant.

## Receipts

Every search produces a receipt — an immutable, signed record of:

- The caller, tenant, and roles at query time
- The exact query, top-k, and filters
- Every source document that contributed, with per-stage scores
- Policy verdicts and their rule ids
- Per-stage timings (embed, vector, BM25, fuse, rerank, policy)

Receipts are first-class; they have their own REST endpoint, a console detail
page, and are how debugging happens. If an answer was wrong, the receipt tells
you which source led the retrieval astray.
