# Engine — Design Notes

**Companion to**: `crates/signet-engine/` in the drop-in.
**Status**: Unverified skeleton, ~3,600 LOC across 14 files + one init migration.
Targets Rust 2024 edition, `sqlx` 0.8, `tokio` 1.42+. Verify locally; the sandbox has no Rust toolchain.

---

## 1. Why two traits

The engine exposes **two** traits:

| Trait | Shape | Consumer |
|---|---|---|
| [`api::EngineHandle`] | Structured: `SearchRequest` / `SearchResponse` and friends | REST endpoints, Rust SDK, future gRPC surface, integration tests |
| [`facade::EngineFacade`] | `Value`-in / `Value`-out primitives | MCP gateway dispatch hot path |

A blanket impl in `facade.rs` gives `EngineFacade` for free to anything that implements `EngineHandle`:

```rust
#[async_trait]
impl<E: EngineHandle> EngineFacade for E { /* ... */ }
```

**Why bother with both?** The gateway already deals in JSON-RPC; wrapping a JSON payload into a `SearchRequest` just to unwrap it again on the other side is wasted work and an extra place for type-drift bugs. The structured API exists for REST and for anyone writing Rust against the engine directly — ergonomics matter there. The facade exists for the MCP hot path, where JSON is the lingua franca.

The two traits cost one file (`facade.rs`, ~275 lines) and remove a class of request-marshalling friction from every MCP call.

---

## 2. Subsystem composition

```
                      ┌──────────────────────────────┐
                      │           Engine             │
                      │  (impl EngineHandle)         │
                      │                              │
                      │  fields:                     │
                      │    config                    │
                      │    registry : SchemaRegistry │
                      │    store    : Store          │
                      │    retriever: Retriever      │
                      │    ingestor : Ingestor       │
                      │    policy   : dyn PolicyEng. │
                      │    receipts : ReceiptStore   │
                      └──┬────────────┬──────────┬───┘
                         │            │          │
              ┌──────────┴──┐   ┌─────┴─────┐   ┌┴──────────┐
              │  Retriever  │   │ Ingestor  │   │ ReceiptSt.│
              └─────┬───────┘   └─────┬─────┘   └─────┬─────┘
                    │                 │                │
         ┌──────────┴─────┐    ┌──────┴──────┐    ┌────┴─────┐
         │ VectorStore    │    │ Embedder    │    │  Store   │
         │ (trait obj)    │    │ (trait obj) │    │ (PgPool) │
         └────────────────┘    └─────────────┘    └──────────┘
```

Every field is `Arc`-wrapped or clone-cheap, so `Engine` is itself `Clone` and shareable across tasks without ceremony. Single `Arc<Engine>` per process.

`Retriever`, `Ingestor`, and `ReceiptStore` are plain structs, not traits — the trait object lives one layer deeper, at `VectorStore` / `Embedder` / `PolicyEngine` / `Reranker`. Those are where implementations genuinely vary; above that, polymorphism is just noise.

---

## 3. Request flow — `search`

```
SearchRequest { caller, namespace, schema, query, top_k, filters }
      │
      ▼
  registry.get(ns, schema)    ──── unknown? → Err(UnknownSchema)
      │
      ▼
  retriever.search(meta, query, top_k, filters)
      │
      │    ┌─ embedder.embed([query]) ─────────┐
      │    ├─ vector.search(collection, …)     │  parallel
      │    └─ postgres ts_vector / ts_rank_cd  │
      │              │
      │              ▼
      │       RRF fusion (alpha-weighted, K=60)
      │              │
      │              ▼
      │       freshness decay
      │              │
      │              ▼
      │       reranker.rerank(query, candidates)
      │              │
      │              ▼
      │       sort desc, truncate to top_k
      ▼
  for each hit: fetch_row_raw(meta, id)
      │
      ▼
  policy.filter(…, PolicyAction::Read, records)  — returns Vec<bool>
      │
      ▼
  receipt persisted: sources, policy verdicts, timings
      │
      ▼
SearchResponse { receipt_id, results }
```

The pipeline's two cost-heavy stages (vector search + BM25 query) run in parallel via `tokio::join!`. If one fails, the other still contributes; we only hard-fail when **both** return empty.

---

## 4. Policy enforcement

Four actions (`Read`, `Write`, `List`, `Delete`). The engine uses:

| Endpoint | Policy action evaluated |
|---|---|
| `search` | `Read` per candidate (filtering happens *after* retrieval) |
| `get` | `Read` on the single record — deny returns `NotFound` so presence is hidden |
| `list` | `List` per candidate |
| `list_relation` | `List` per candidate on the **target** schema |
| `aggregate` | `List` once on the parent (counts reveal less than records) |

The `PolicyEngine` trait supports both a single-record `evaluate` and a batch `filter`. Default v0.1 impl is `AllowAllPolicy` with a warning log on construction. When CSL starts emitting policy WASM (§13 of the CSL spec, currently deferred), `WasmPolicy` replaces it — same trait, no call-site changes.

Denied reads return `NotFound` rather than `PolicyDenied` on purpose: leaking "this record exists but you can't see it" is a side-channel. `list` filters silently. `search` filters silently and persists `deny` verdicts on the receipt for audit.

---

## 5. Hybrid retrieval

### Backends
- **Vector**: `signet_vectorstore::VectorStore::search`. In dev, `InMemoryStore`; in production, `QdrantStore`.
- **BM25**: Postgres full-text (`to_tsvector` + `ts_rank_cd`). Not strictly BM25, but close enough for v0.1. Tantivy-backed replacement planned for v0.2.

### Fusion
Reciprocal Rank Fusion (Cormack et al. 2009). For each candidate, rank-based score `1 / (K + rank + 1)` with `K = 60`, weighted by `alpha` (vector side) and `1 - alpha` (BM25 side). Schema-level override via CSL's `context { hybrid_alpha = ... }` — falls back to `EngineConfig::retrieval::default_hybrid_alpha` (0.6).

RRF is scale-invariant — vector cosines and BM25 raw scores can't be compared directly, so fusing the raw numbers is meaningless. Rank fusion sidesteps the problem.

### Freshness
Applied **after** fusion. Three strategies: `Exponential { half_life_secs }`, `Linear { window_secs }`, `Step { cliffs, factors }`. Schema-configurable; `None` by default. Each candidate's age (from `created_at`) maps to a factor in `[0, 1]`; the factor multiplies the fused score.

### Reranker
Applied to the top `candidate_k` (default 64) candidates. `IdentityReranker` is the default; `HttpReranker` talks to any OpenAI-compatible `/v1/rerank` endpoint (Cohere, Voyage, Jina, BGE-via-TEI). Feature-gated on `reranker` to keep `reqwest` optional.

---

## 6. Ingestion pipeline

Input: `signet_connector_sdk::Resource { id, kind, title, body, metadata, permissions, source_updated_at }`.

Steps:
1. `upsert_row` → Postgres (`id`, `title`, `body`, `created_at`, `metadata` JSONB). v0.1 uses a narrow column set; the full typed projection lands with the CSL→Rust struct codegen path in v0.2.
2. `chunk_body` per the schema's `ChunkingStrategy`. Three strategies: `Fixed`, `Semantic` (paragraph-aware with overlap), `Recursive` (priority separator list). Token counts use the ~4-chars-per-token heuristic; a real tokenizer (tiktoken, tokenizers) lands later.
3. `embedder.embed(chunks)` in one batch.
4. `vector_store.upsert(collection, chunks)` — each chunk carries `record_id`, `seq`, and `created_at` in its metadata so retrieval can decay and deduplicate.

No row is considered "committed" until both the row and its chunks are written. v0.2 will wrap this in a saga with explicit rollback on partial failure; v0.1 assumes eventual consistency is acceptable.

---

## 7. Receipts

Every `search`, `get`, and `aggregate` call persists one receipt:

```rust
Receipt {
    id,
    caller,
    qualified_schema,      // "acme.crm.Customer"
    tool,                  // "search_customer"
    arguments,             // echoed request args
    sources,               // Vec<SourceRef { schema, record_id, chunk_id?, score }>
    policies_applied,      // Vec<PolicyRef { schema, predicate, verdict }>
    timings_ms,            // Timings { embed, retrieval, rerank, policy, total }
    created_at,
}
```

`list` and `list_relation` don't log receipts in v0.1 (they're high-volume and mostly uninteresting from an audit standpoint). That policy is revisitable.

Receipts persist in `signet_receipts` with indexes on `caller_id`, `created_at`, and `qualified_schema`. Retention is 90 days by default; a `ReceiptStore::prune` method is provided for the nightly cleanup cron.

Enterprise Edition will sign receipts with Ed25519. The hook is `ReceiptConfig::sign`; v0.1 ignores it.

---

## 8. Migrations and the schema registry

Two migration paths, deliberately separate:

1. **Engine control-plane tables** (`signet_schemas`, `signet_connectors`, `signet_receipts`, `signet_ingest_state`, `signet_lineage`, `signet_mcp_sessions`) — live in `migrations/` and run via `sqlx::migrate!` on startup.
2. **Per-CSL-schema tables** (`customer`, `ticket`, `customer_chunk`, etc.) — emitted by `signet_csl::codegen::sql` into the compile output directory and applied via `Store::apply_schema_ddl` when `signet schema apply` runs.

Keeping them separate means engine upgrades don't touch user data, and user schema changes don't require an engine restart.

The `SchemaRegistry` loads `SchemaMeta` from `<compile_dir>/schemas/*.json` at boot. The format is a flat JSON per schema — the CSL compiler writes it alongside the SQL migration and the MCP tool descriptor.

---

## 9. Known gaps — what to wire before production

Listed in roughly decreasing priority:

1. **Policy WASM.** `AllowAllPolicy` lets everything through. `WasmPolicy` is a placeholder struct. Nothing ships to external customers before this is real.
2. **Typed row mapping.** v0.1 stores and reads `(id, title, body, created_at, metadata)` only. Full per-schema projection (all declared CSL fields materialized into typed columns) needs CSL→Rust codegen and generated `sqlx::query_as!` macros.
3. **Vector-store filter passthrough.** `retrieval::vector_search` currently passes `None` for filters and post-filters in the RRF merge. Native pushdown (`qdrant::Filter`) cuts latency significantly on large collections.
4. **Cursor encoding.** `list` and `list_relation` return the last-seen `id` as an opaque cursor. Production needs proper cursor encoding with a secret-keyed HMAC to prevent cursor forging.
5. **Tokenizer.** The "~4 chars = 1 token" heuristic in `ingest::chunk_body` is fine for dev but off by a meaningful factor on CJK and source code. Plug in `tokenizers` (HF) or `tiktoken-rs`.
6. **Tantivy BM25.** Postgres `ts_rank_cd` is approximate. A Tantivy-backed index per schema gives real BM25 and dramatically better multi-term recall.
7. **Hot reload.** The registry is immutable after `Engine::new`. A file-watcher in `SchemaRegistry` plus a read-write lock lets `signet schema apply` take effect without restart.
8. **Receipt signing.** Ed25519 over the canonical JSON of the receipt body. Key rotation, verification SDK, and customer-controllable key material all need design.
9. **`list` receipts.** Currently skipped for volume reasons. Decide the right sampling policy (every Nth, every call over a facet filter, …).
10. **Relation walking.** Only `many` relations are supported. `one` relations (inverse-direction lookup) need a different code path and ideally a join.

---

## 10. Verification checklist (first `cargo check` pass)

This code has not been compiled. The most likely surface for adjustments:

1. **`sqlx::query_as` type parameters.** I used tuple types throughout — e.g. `(String, Option<String>, …)`. sqlx 0.8 handles this via its `FromRow` derive, but tuples should work. If errors mention `FromRow` bounds, wrap the tuple in a named struct with `#[derive(sqlx::FromRow)]`.

2. **`time::OffsetDateTime` column binding.** Requires `features = ["time"]` on the `sqlx` dependency — added in `Cargo.toml`. If you see unresolved trait bounds on `time::OffsetDateTime`, double-check the feature is active.

3. **`async_trait` on traits with `&self` generics.** Native async-fn-in-trait is stable on 1.75+ but has stricter `Send` bounds. Keeping `#[async_trait]` avoids that for v0.1.

4. **`dashmap::DashMap::iter()` lifetimes.** In `SchemaRegistry::iter`, I collect into a `Vec` inside the method. That's correct — `dashmap`'s iterator holds a read guard and mustn't escape the function.

5. **`serde_json::Value::as_str()` vs `Value::as_str`.** I used both styles. Either works; clippy will flag inconsistency, not correctness.

6. **Gateway's `AppState`.** I added an `engine: Arc<dyn EngineFacade>` field. The existing `rest.rs` uses `Router<AppState>` as a type parameter without touching the field, so no changes needed there. If you add handlers that pull from state, they use the standard `State<AppState>` extractor.

7. **Binary `signet-gateway`.** It now imports `signet_vectorstore::memory::InMemoryStore`, `signet_vectorstore::qdrant::QdrantStore`, `signet_embedders::stub::StubEmbedder`. If those paths differ in the actual crates, the binary won't link — fix the paths.

8. **Testcontainers / integration tests.** None included here; the acceptance test in the scaffolding brief's Phase 12 is what exercises this crate end-to-end.

---

## 11. Minimum runnable example

Once the workspace builds:

```rust
use std::sync::Arc;
use signet_engine::{Engine, EngineConfig};
use signet_vectorstore::memory::InMemoryStore;
use signet_embedders::stub::StubEmbedder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = EngineConfig::test();
    let vector_store = Arc::new(InMemoryStore::default());
    let embedder     = Arc::new(StubEmbedder::new(64));

    let engine = Engine::new_dev(config, vector_store, embedder).await?;

    // Now impls both EngineHandle (structured) and EngineFacade (JSON).
    let caller = signet_engine::api::Caller::test("u_dev");
    // Register a schema, ingest a resource, run a search — see the
    // scaffolding brief's Phase 11 "engineering-context" example.
    Ok(())
}
```

The `signet-gateway` binary does exactly this composition (in `bin/server.rs`) and then wraps the `Engine` as `Arc<dyn EngineFacade>` before handing it to `build_app`.

*End of engine design notes.*
