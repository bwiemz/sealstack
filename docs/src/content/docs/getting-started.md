---
title: Getting started
description: Stand up a local ContextForge gateway, register a schema, ingest some data, and run your first query.
---

This guide gets you from nothing to a working ContextForge gateway serving MCP
tools and REST endpoints in about ten minutes. Everything runs locally with
Docker Compose.

## Prerequisites

- Docker Desktop (or Docker Engine + Compose plugin)
- Rust 1.87+ — only required if you're building the CLI from source. Pre-built
  binaries are attached to each GitHub release.
- `cfg` CLI on your `$PATH`.

## 1. Start the dev stack

Clone the repo and bring up Postgres, Qdrant, and the gateway:

```bash
git clone https://github.com/contextforge/contextforge.git
cd contextforge
docker compose -f deploy/docker/compose.dev.yaml up -d
```

You should see the gateway come up on port `7070`, Postgres on `5432`, and
Qdrant on `6333`/`6334`.

Verify the gateway is reachable:

```bash
curl -s http://localhost:7070/healthz
# {"status":"ok"}
```

## 2. Define a schema

Create `schemas/doc.csl`:

```
namespace examples

schema Doc v1 {
  id:    ID @primary
  title: String @indexed
  body:  Text @searchable @chunked

  context {
    embedder        = "stub"
    vector_dims     = 64
    chunking        = semantic(max_tokens = 400, overlap = 40)
    freshness_decay = exponential(half_life = 30d)
  }
}
```

Compile and apply it. The CLI turns CSL into a JSON schema meta the gateway
understands and POSTs it via `/v1/schemas`:

```bash
cfg schema apply schemas/doc.csl
```

## 3. Register a connector

Point the local-files connector at a directory of markdown:

```bash
mkdir -p sample-docs
echo "# Setup\nUse Postgres 16." > sample-docs/setup.md
cfg connector add local-files --schema examples.Doc --root ./sample-docs
```

Then trigger a sync:

```bash
cfg connector sync local-files/examples.Doc
```

## 4. Query

```bash
cfg query --schema examples.Doc "what does the setup doc say about postgres?"
```

You'll get hits plus a receipt id. Fetch the receipt to see why each source
surfaced:

```bash
cfg receipt show <receipt-id>
```

## 5. Next

- **[CSL Tutorial](/csl/tutorial)** — model a multi-schema domain
- **[MCP Tools](/api/mcp)** — connect Claude, Cursor, or any MCP-compatible client
- **[Deployment](/ops/deployment)** — move off Compose to a real Kubernetes install
