---
title: CSL tutorial
description: Model a multi-schema domain in CSL and walk through what the compiler generates.
---

CSL — the **Context Schema Language** — is the DSL you declare data shapes in.
A CSL file gets compiled to Postgres DDL, MCP tool descriptors, TypeScript and
Python SDK clients, and a JSON schema-meta document the engine reads at runtime.

## A minimal schema

```
namespace acme.crm

schema Customer v1 {
  id:      ID @primary
  name:    String @indexed
  plan:    String @facet
  notes:   Text   @searchable @chunked

  context {
    embedder        = "openai/text-embedding-3-small"
    vector_dims     = 1536
    chunking        = semantic(max_tokens = 400, overlap = 40)
    freshness_decay = exponential(half_life = 90d)
  }
}
```

## Decorators

| Decorator      | Meaning                                                      |
|----------------|--------------------------------------------------------------|
| `@primary`     | The field is this schema's primary key.                      |
| `@indexed`     | Create a B-tree index on the column.                         |
| `@unique`      | Enforce uniqueness at the DB level.                          |
| `@searchable`  | Include the column in BM25's tsvector.                       |
| `@chunked`     | Feed the column into the embedder for vector search.         |
| `@facet`       | Expose as a filterable + aggregatable facet.                 |
| `@pii`         | Mark the column as PII — policy rules can scope access.      |

## Relations

```
schema Ticket v1 {
  id:       ID @primary
  customer: Ref<Customer>
  subject:  String @indexed
  body:     Text @searchable @chunked
  ...
}
```

`Ref<Customer>` emits a foreign key plus a `customer_id` column, and an
auto-generated `list_customer_tickets` MCP tool that paginates tickets for
any customer.

## Context block

The `context { ... }` block configures retrieval for this schema:

- `embedder` — which embedder backend to use. Must match a backend enabled on the engine.
- `vector_dims` — output dimension of the embedder. Must match the vector store collection.
- `chunking` — strategy: `semantic(max_tokens, overlap)`, `fixed(size)`, or `recursive(separators, max_tokens)`.
- `freshness_decay` — how to discount older content at query time: `none`, `exponential(half_life)`, `linear(window)`, `step(cliffs, factors)`.
- `default_top_k` — default top-k when the caller doesn't supply one.

## Compile

```bash
sealstack compile schemas/
```

Produces:

- `out/sql/NNNN_up.sql` — Postgres forward migration
- `out/schemas/<ns>.<name>.schema.json` — runtime schema meta for the gateway
- `out/mcp/<ns>.<name>.tools.json` — MCP tool descriptors
- `out/sdk/ts/*`, `out/sdk/py/*` — typed SDK clients (optional)

## Apply

```bash
sealstack schema apply schemas/
```

Runs the DDL against Postgres and POSTs each schema meta to
`POST /v1/schemas` on the gateway. The gateway persists them, so a restart
is transparent.
