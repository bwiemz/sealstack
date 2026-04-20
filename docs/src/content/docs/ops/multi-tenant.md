---
title: Multi-tenant isolation
description: How ContextForge scopes queries so tenants never see each other's data.
---

## Model

Every per-schema table the CSL compiler emits carries a `tenant text` column.
Rows inserted by the ingest pipeline are stamped with the tenant of the
connector binding that produced them. Rows inserted via the REST / MCP path
inherit the tenant of the authenticated caller.

At query time, the retrieval layer filters with:

```sql
WHERE coalesce(tenant,'') = $caller_tenant
```

The vector store is filtered the same way, via a `tenant` payload key
injected at ingest time. A caller whose tenant is empty only sees rows with
no tenant set — the default-tenant case for single-tenant deployments.

## How the tenant is established

The caller's tenant comes from their identity:

- **MCP routes** — the `tenant` claim on the validated JWT, courtesy of the
  OAuth bearer middleware at [`src/auth.rs`](https://github.com/bwiemz/contextforge/blob/main/crates/cfg-gateway/src/auth.rs).
  If the gateway runs in `AuthMode::Disabled` (dev), the tenant is empty
  string.
- **REST routes** — either the authenticated JWT (when present) or the
  `X-Cfg-Tenant` header (dev fallback).

## Boundaries and limits

- **Connector bindings are single-tenant.** A binding is created against
  exactly one tenant; resources that binding produces all end up under that
  tenant. If you need cross-tenant ingestion, register one binding per tenant.
- **Search is always scoped.** There is no "superadmin" query path. Even the
  console UI is just a caller with its own identity.
- **Receipts record the caller.** Every receipt carries the tenant the query
  ran under, so audit trails are per-tenant by construction.

## Anti-pitfalls

- Don't name a CSL field `tenant` unless you really want to override the
  injected column. The compiler detects a pre-existing `tenant` field and
  skips the auto-insert, but you lose the default-tenant semantics if you
  don't keep the column type as `text NOT NULL DEFAULT ''`.
- Don't assume pg row-level security is enough. RLS works, but the BM25 SQL
  path uses `coalesce(tenant,'') = $n` explicitly so the filter is visible in
  query plans and can't be accidentally bypassed by a superuser role.
