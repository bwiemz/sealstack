---
title: Deployment
description: Run ContextForge in production.
---

## Docker Compose (dev + small prod)

The fastest path:

```bash
docker compose -f deploy/docker/compose.dev.yaml up -d
```

## Kubernetes (Helm — coming in v0.2)

Helm chart scaffolding lives in [`deploy/helm/`](https://github.com/bwiemz/contextforge/tree/main/deploy/helm).
Not production-ready yet; track the [v0.2 milestone](https://github.com/bwiemz/contextforge/milestones).

## Gateway image

The gateway image is published on every tagged release:

```
ghcr.io/bwiemz/contextforge/gateway:<version>
ghcr.io/bwiemz/contextforge/gateway:latest
```

Multi-arch (linux/amd64, linux/arm64).

## Required services

ContextForge needs two backends:

- **Postgres 16** — rows + receipts + full-text search. Pinned to 16 for the
  `ts_rank_cd` scoring behavior; older versions will work but with slightly
  different BM25 ranking.
- **Qdrant** (latest) — vector store. Accessible over gRPC on `:6334`.

Optional:

- **Redis** — session store and rate-limit counters. Without it, sessions
  spill to Postgres (same `cfg_mcp_sessions` table).
