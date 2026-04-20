---
title: Configuration
description: Environment variables the gateway understands.
---

The gateway is configured entirely via environment variables. The CLI reads
its own config from `~/.signet/config.toml`.

## Core

| Variable            | Default                                 | Purpose                                  |
|---------------------|-----------------------------------------|------------------------------------------|
| `SIGNET_BIND`          | `0.0.0.0:7070`                          | Listen address.                          |
| `SIGNET_DATABASE_URL`  | `postgres://signet:signet@localhost:5432/signet` | Postgres connection.                     |
| `SIGNET_QDRANT_URL`    | `http://localhost:6334`                 | Qdrant gRPC. Empty ⇒ in-memory.          |
| `SIGNET_REDIS_URL`     | unset                                   | Optional Redis for sessions + ratelimit. |
| `RUST_LOG`          | `info`                                  | Tracing filter.                          |

## Embedder selection

| Variable                 | Default                      | Purpose                                          |
|--------------------------|------------------------------|--------------------------------------------------|
| `SIGNET_EMBEDDER`           | `stub`                       | `stub`, `openai`, or `voyage`.                   |
| `SIGNET_EMBEDDER_MODEL`     | per-backend default          | Model id.                                        |
| `SIGNET_EMBEDDER_ENDPOINT`  | vendor default               | Override for proxies / self-hosted TEI.          |
| `SIGNET_EMBEDDER_DIMS`      | model default                | Output vector dims — must match schema's.        |
| `OPENAI_API_KEY`         | —                            | Required for `openai`.                           |
| `VOYAGE_API_KEY`         | —                            | Required for `voyage`.                           |

## Reranker selection

| Variable                | Default    | Purpose                                |
|-------------------------|------------|----------------------------------------|
| `SIGNET_RERANKER`          | `identity` | `identity` or `http`.                  |
| `SIGNET_RERANKER_URL`      | —          | Required for `http`.                   |
| `SIGNET_RERANKER_MODEL`    | `rerank-default` | Model id echoed in request payload. |
| `SIGNET_RERANKER_API_KEY`  | —          | Optional bearer auth.                  |

## Authentication (MCP routes)

| Variable                 | Default     | Purpose                                          |
|--------------------------|-------------|--------------------------------------------------|
| `SIGNET_AUTH_MODE`          | `disabled`  | `disabled` or `hs256`.                           |
| `SIGNET_AUTH_HS256_SECRET`  | —           | Required for `hs256`.                            |
| `SIGNET_AUTH_ISSUERS`       | —           | CSV of accepted `iss` claims. Empty = any.       |
| `SIGNET_AUTH_AUDIENCES`     | —           | CSV of accepted `aud` claims. Empty = skip aud.  |

## OAuth metadata

| Variable             | Default                                        | Purpose                                 |
|----------------------|------------------------------------------------|-----------------------------------------|
| `SIGNET_PUBLIC_URL`     | `http://localhost:7070`                        | Published as the "resource" URL.        |
| `SIGNET_OAUTH_ISSUER`   | `http://localhost:8080/realms/signet`    | Advertised authorization server URL.    |
