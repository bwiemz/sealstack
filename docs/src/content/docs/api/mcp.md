---
title: MCP tools
description: The tools every SealStack schema registers on the MCP endpoint.
---

SealStack speaks MCP 2025-11-25 streamable HTTP at `/mcp/:server_name`. For
every registered CSL schema, the gateway auto-registers a fixed set of tools:

| Tool                             | Kind            | Purpose                                           |
|----------------------------------|-----------------|---------------------------------------------------|
| `search_<ns>_<Schema>`           | Search          | Hybrid BM25 + vector search.                      |
| `get_<ns>_<Schema>`              | Get             | Fetch one row by primary key.                     |
| `list_<ns>_<Schema>`             | List            | Cursor-paginated listing with filters.            |
| `list_<ns>_<Schema>_<relation>`  | ListRelation    | Walk a `@many` relation from a parent.            |
| `aggregate_<ns>_<Schema>_<facet>`| Aggregate       | Histogram over a `@facet` field.                  |

## Tool-name sanitization

MCP restricts tool names to `^[a-zA-Z0-9_-]{1,64}$`. CSL namespaces (`acme.crm`)
contain dots, so the gateway rewrites them to underscores when building tool
names. `acme.crm.Customer` becomes `search_acme_crm_Customer`. The dotted form
is preserved in the tool's `title` and `description` for human readers.

## Session lifecycle

1. Client POSTs an `initialize` request to `/mcp/<server_name>`.
2. Gateway returns an `Mcp-Session-Id` header; the client passes it on every
   subsequent request.
3. `tools/list` enumerates the auto-registered tools.
4. `tools/call` dispatches into the engine's JSON-shaped facade.

## Authentication

By default (`SEALSTACK_AUTH_MODE=disabled`), every request is accepted with an
`anonymous` caller. In production, set `SEALSTACK_AUTH_MODE=hs256` and
`SEALSTACK_AUTH_HS256_SECRET` — the middleware validates `Authorization: Bearer <jwt>`
on every MCP request and rejects unauthenticated calls with a 401 plus a
`WWW-Authenticate: Bearer ... resource_metadata="..."` header pointing at the
OAuth 2.1 discovery document at `/.well-known/oauth-protected-resource`.

## Example: Claude Desktop

Add to Claude Desktop's MCP server config:

```json
{
  "mcpServers": {
    "sealstack": {
      "url": "https://cfg.acme.internal/mcp/default",
      "transport": "http",
      "auth": {
        "type": "oauth",
        "resource": "https://cfg.acme.internal"
      }
    }
  }
}
```
