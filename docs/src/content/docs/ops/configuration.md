---
title: Configuration
description: Environment variables the gateway understands.
---

The gateway is configured entirely via environment variables. The CLI reads
its own config from `~/.sealstack/config.toml`.

## Core

| Variable            | Default                                 | Purpose                                  |
|---------------------|-----------------------------------------|------------------------------------------|
| `SEALSTACK_BIND`          | `0.0.0.0:7070`                          | Listen address.                          |
| `SEALSTACK_DATABASE_URL`  | `postgres://sealstack:sealstack@localhost:5432/sealstack` | Postgres connection.                     |
| `SEALSTACK_QDRANT_URL`    | `http://localhost:6334`                 | Qdrant gRPC. Empty ⇒ in-memory.          |
| `SEALSTACK_REDIS_URL`     | unset                                   | Optional Redis for sessions + ratelimit. |
| `RUST_LOG`          | `info`                                  | Tracing filter.                          |

## Embedder selection

| Variable                 | Default                      | Purpose                                          |
|--------------------------|------------------------------|--------------------------------------------------|
| `SEALSTACK_EMBEDDER`           | `stub`                       | `stub`, `openai`, or `voyage`.                   |
| `SEALSTACK_EMBEDDER_MODEL`     | per-backend default          | Model id.                                        |
| `SEALSTACK_EMBEDDER_ENDPOINT`  | vendor default               | Override for proxies / self-hosted TEI.          |
| `SEALSTACK_EMBEDDER_DIMS`      | model default                | Output vector dims — must match schema's.        |
| `OPENAI_API_KEY`         | —                            | Required for `openai`.                           |
| `VOYAGE_API_KEY`         | —                            | Required for `voyage`.                           |

## Reranker selection

| Variable                | Default    | Purpose                                |
|-------------------------|------------|----------------------------------------|
| `SEALSTACK_RERANKER`          | `identity` | `identity` or `http`.                  |
| `SEALSTACK_RERANKER_URL`      | —          | Required for `http`.                   |
| `SEALSTACK_RERANKER_MODEL`    | `rerank-default` | Model id echoed in request payload. |
| `SEALSTACK_RERANKER_API_KEY`  | —          | Optional bearer auth.                  |

## Authentication (MCP routes)

| Variable                 | Default     | Purpose                                          |
|--------------------------|-------------|--------------------------------------------------|
| `SEALSTACK_AUTH_MODE`          | `disabled`  | `disabled` or `hs256`.                           |
| `SEALSTACK_AUTH_HS256_SECRET`  | —           | Required for `hs256`.                            |
| `SEALSTACK_AUTH_ISSUERS`       | —           | CSV of accepted `iss` claims. Empty = any.       |
| `SEALSTACK_AUTH_AUDIENCES`     | —           | CSV of accepted `aud` claims. Empty = skip aud.  |

## OAuth metadata

| Variable             | Default                                        | Purpose                                 |
|----------------------|------------------------------------------------|-----------------------------------------|
| `SEALSTACK_PUBLIC_URL`     | `http://localhost:7070`                        | Published as the "resource" URL.        |
| `SEALSTACK_OAUTH_ISSUER`   | `http://localhost:8080/realms/sealstack`    | Advertised authorization server URL.    |
