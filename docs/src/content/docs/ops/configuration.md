---
title: Configuration
description: Environment variables the gateway understands.
---

The gateway is configured entirely via environment variables. The CLI reads
its own config from `~/.cfg/config.toml`.

## Core

| Variable            | Default                                 | Purpose                                  |
|---------------------|-----------------------------------------|------------------------------------------|
| `CFG_BIND`          | `0.0.0.0:7070`                          | Listen address.                          |
| `CFG_DATABASE_URL`  | `postgres://cfg:cfg@localhost:5432/cfg` | Postgres connection.                     |
| `CFG_QDRANT_URL`    | `http://localhost:6334`                 | Qdrant gRPC. Empty ⇒ in-memory.          |
| `CFG_REDIS_URL`     | unset                                   | Optional Redis for sessions + ratelimit. |
| `RUST_LOG`          | `info`                                  | Tracing filter.                          |

## Embedder selection

| Variable                 | Default                      | Purpose                                          |
|--------------------------|------------------------------|--------------------------------------------------|
| `CFG_EMBEDDER`           | `stub`                       | `stub`, `openai`, or `voyage`.                   |
| `CFG_EMBEDDER_MODEL`     | per-backend default          | Model id.                                        |
| `CFG_EMBEDDER_ENDPOINT`  | vendor default               | Override for proxies / self-hosted TEI.          |
| `CFG_EMBEDDER_DIMS`      | model default                | Output vector dims — must match schema's.        |
| `OPENAI_API_KEY`         | —                            | Required for `openai`.                           |
| `VOYAGE_API_KEY`         | —                            | Required for `voyage`.                           |

## Reranker selection

| Variable                | Default    | Purpose                                |
|-------------------------|------------|----------------------------------------|
| `CFG_RERANKER`          | `identity` | `identity` or `http`.                  |
| `CFG_RERANKER_URL`      | —          | Required for `http`.                   |
| `CFG_RERANKER_MODEL`    | `rerank-default` | Model id echoed in request payload. |
| `CFG_RERANKER_API_KEY`  | —          | Optional bearer auth.                  |

## Authentication (MCP routes)

| Variable                 | Default     | Purpose                                          |
|--------------------------|-------------|--------------------------------------------------|
| `CFG_AUTH_MODE`          | `disabled`  | `disabled` or `hs256`.                           |
| `CFG_AUTH_HS256_SECRET`  | —           | Required for `hs256`.                            |
| `CFG_AUTH_ISSUERS`       | —           | CSV of accepted `iss` claims. Empty = any.       |
| `CFG_AUTH_AUDIENCES`     | —           | CSV of accepted `aud` claims. Empty = skip aud.  |

## OAuth metadata

| Variable             | Default                                        | Purpose                                 |
|----------------------|------------------------------------------------|-----------------------------------------|
| `CFG_PUBLIC_URL`     | `http://localhost:7070`                        | Published as the "resource" URL.        |
| `CFG_OAUTH_ISSUER`   | `http://localhost:8080/realms/contextforge`    | Advertised authorization server URL.    |
