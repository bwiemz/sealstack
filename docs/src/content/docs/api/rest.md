---
title: REST reference
description: Gateway REST endpoints.
---

All JSON responses follow an envelope:

```json
{ "data": <T>, "error": null }
```

On failure:

```json
{ "data": null, "error": { "code": "<code>", "message": "<human>" } }
```

## Health

| Method | Path       | Purpose                |
|--------|------------|------------------------|
| `GET`  | `/healthz` | Liveness — always 200. |
| `GET`  | `/readyz`  | Readiness.             |

## Query

| Method | Path         | Body                                                          |
|--------|--------------|---------------------------------------------------------------|
| `POST` | `/v1/query`  | `{ "schema": "ns.Name", "query": "...", "top_k": 10, "filters": {} }` |

Returns a `SearchResponse` with hits, a receipt id, and elapsed ms.

## Schemas

| Method | Path                                 | Purpose                                          |
|--------|--------------------------------------|--------------------------------------------------|
| `GET`  | `/v1/schemas`                        | List every registered schema (summary).          |
| `POST` | `/v1/schemas`                        | Register a compiled schema. Body: `{ meta }`.    |
| `GET`  | `/v1/schemas/:qualified`             | Full schema metadata.                            |
| `POST` | `/v1/schemas/:qualified/ddl`         | Apply a DDL bundle produced by `cfg-csl`.        |

Schemas registered via `POST /v1/schemas` persist to Postgres and survive a
gateway restart. The in-memory `SchemaRegistry` is rehydrated from the DB at
boot.

## Connectors

| Method | Path                                   | Purpose                    |
|--------|----------------------------------------|----------------------------|
| `GET`  | `/v1/connectors`                       | List registered bindings.  |
| `POST` | `/v1/connectors`                       | Register a binding.        |
| `POST` | `/v1/connectors/:id/sync`              | Run one-shot sync.         |

`POST /v1/connectors` body:

```json
{
  "kind": "local-files",
  "schema": "acme.Doc",
  "config": { "root": "/srv/docs" }
}
```

## Receipts

| Method | Path                 | Purpose           |
|--------|----------------------|-------------------|
| `GET`  | `/v1/receipts/:id`   | Fetch a receipt.  |

## Error codes

| Code               | HTTP | Meaning                                         |
|--------------------|------|-------------------------------------------------|
| `invalid_argument` | 400  | Body shape or field value rejected.             |
| `unknown_schema`   | 404  | Schema not registered.                          |
| `not_found`        | 404  | Resource (receipt, connector, row) not found.   |
| `policy_denied`    | 403  | CSL policy rule rejected the caller.            |
| `backend`          | 500  | Postgres, Qdrant, or embedder error.            |
| `internal`         | 500  | Uncategorized — check gateway logs.             |
