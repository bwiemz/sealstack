# Ingestion — Design Notes

**Companion to**: `crates/cfg-connector-sdk/`, `crates/cfg-ingest/`, and `connectors/local-files/` in the drop-in.
**Status**: Unverified skeleton. 1,368 lines across 7 files. Targets `notify` 7, `walkdir` 2.5, Rust 2024. Verify locally.

---

## 1. The split

Three crates, three concerns:

| Crate | Role | Depends on |
|---|---|---|
| `cfg-connector-sdk` | Defines the `Connector` trait and the `Resource` / `ChangeEvent` shapes | `cfg-common` |
| `cfg-ingest` | Runtime that drives connectors and feeds their output into the engine | `cfg-engine`, `cfg-connector-sdk` |
| `connectors/local-files` | A concrete `Connector` implementation backed by a filesystem directory | `cfg-connector-sdk` |

Three crates rather than one because each has a different consumer:

- Connector authors depend on just the SDK. A third party writing a SharePoint or Jira connector shouldn't have to pull in the engine or the ingest runtime.
- The ingest runtime is an internal engine subsystem — nobody outside ContextForge calls it directly.
- Concrete connectors are binary shims that live under `connectors/` per the workspace layout in the scaffolding brief, not under `crates/`. They're treated like third-party contributions even when we write them.

---

## 2. `cfg-connector-sdk` — the trait

The whole SDK is one trait:

```rust
#[async_trait]
pub trait Connector: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    async fn list(&self) -> CfgResult<ResourceStream>;
    async fn fetch(&self, id: &ResourceId) -> CfgResult<Resource>;
    async fn subscribe(&self) -> CfgResult<Option<ChangeStream>> { Ok(None) }
    async fn healthcheck(&self) -> CfgResult<()> { Ok(()) }
}
```

### Stream types

`ResourceStream = Pin<Box<dyn Stream<Item = Resource> + Send>>` and `ChangeStream` is the analogous alias for push updates. Aliasing means connector implementations don't have to type out the full boxed-stream shape in every signature, and the `change_streams` helpers (`resource_stream(Vec<Resource>)`, `change_stream(Vec<ChangeEvent>)`) handle the common case of "I have a `Vec`, give me a stream."

### Resource

```rust
pub struct Resource {
    pub id: ResourceId,
    pub kind: String,
    pub title: Option<String>,
    pub body: String,
    pub metadata: serde_json::Map<String, Value>,
    pub permissions: Vec<PermissionPredicate>,
    pub source_updated_at: time::OffsetDateTime,
}
```

`ResourceId` is a thin wrapper over `String` — opaque to the engine, meaningful only within the producing connector. Absolute file paths for `local-files`, SSO group IRIs for GitHub, `thread_ts` for Slack. The engine never parses them.

### Permissions

`PermissionPredicate { principal, action }` is **source-side** metadata, not the authoritative access-control rule. The engine's runtime policy comes from the CSL `policy { }` block, compiled to WASM. We attach source permissions to the resource so receipts can display them — an auditor can see "Slack said this message was visible to @eng-core and our CSL policy agreed" — but we do not enforce them at retrieval time.

This is deliberate. Source-side permissions drift; they are not a reliable source of truth for who should see what in an AI-mediated workflow. CSL policies are the authoritative layer.

### Change events

```rust
pub enum ChangeEvent {
    Upsert(Resource),
    Delete(ResourceId),
}
```

Thin. Enough for the common push patterns (webhook arrives → decide upsert or delete → feed into engine). Richer event types (move, rename, permission-only change) can be added without a breaking change.

---

## 3. `cfg-ingest` — the runtime

### Binding

```rust
pub struct ConnectorBinding {
    pub connector: Arc<dyn Connector>,
    pub target_namespace: String,
    pub target_schema: String,
    pub interval: Option<Duration>,
}
```

A binding is the unit the runtime operates on: one connector plus one target schema. Bindings are keyed by `"<connector_name>/<namespace>.<schema>"` — the connector name alone isn't enough because a single connector can legitimately write to multiple schemas (e.g. `github` → `Ticket` for issues and `github` → `PullRequest` for PRs).

### Three modes

1. **`sync_once(binding_id)`** — one-shot. The CLI's `cfg connector sync <id>` and the integration-test harness both call this. Returns a `SyncOutcome` with counters for resources seen / ingested / failed plus elapsed time.

2. **`start_background()`** — spawns one Tokio task per binding that has an `interval` configured. The poll loop calls `sync_once` on the tick and logs the outcome. Missed ticks use `MissedTickBehavior::Delay` so a slow sync doesn't pile up.

3. **Subscribe loops** — if a connector returns `Some(stream)` from `subscribe()`, a dedicated task forwards `ChangeEvent`s into the engine as they arrive. Independent of the poll interval; push and pull coexist.

### Error handling

Three levels:

- **Binding lookup fails** → `SyncOutcomeKind::NotFound`. Not retried.
- **`list()` fails** → `SyncOutcomeKind::FailedList`. Logged, counted, retried on the next tick.
- **Per-resource ingestion fails** → counted in `resources_failed`, logged at `warn`, sync continues. One bad file does not stop the run.

The rationale: sync is a batch operation over N items. Making the whole sync fail because of one bad item turns partial progress into zero progress. We'd rather ingest 99% and flag the 1%.

### What's not in v0.1

- **Incremental sync.** `cfg_ingest_state` is created by the engine's migration but not read — every `sync_once` walks the full source. Idempotent upserts mean this is correct, just wasteful at scale.
- **Dead-letter handling.** Per-resource failures go to the log. A real DLQ needs persistence and a replay mechanism.
- **Rate limiting.** No backoff when the source returns `429`. Vendor-specific concern best handled at the connector level.
- **Metrics.** `SyncOutcome` captures per-run numbers but doesn't expose them to Prometheus. A `tracing` → OpenTelemetry bridge covers most of this generically.

---

## 4. `local-files` — the reference connector

### What it does

Walks the configured root (canonicalized eagerly via `std::fs::canonicalize`), yields one `Resource` per file whose extension is in the supported set. Each resource carries:

| Field | Source |
|---|---|
| `id` | Absolute canonical path |
| `kind` | Lowercased file extension (or `"text"`) |
| `title` | Filename without extension |
| `body` | Full file contents as UTF-8 |
| `metadata.path` | Canonical path (redundant with id; convenient for filters) |
| `metadata.size_bytes` | From the file's `metadata()` |
| `metadata.filename` | Original filename |
| `permissions` | `[{ principal: "*", action: "read" }]` |
| `source_updated_at` | File `mtime` as `OffsetDateTime` |

### Why eager canonicalization

The connector's root is the security perimeter. Canonicalizing at construction time means every subsequent path comparison (`path.starts_with(&self.root)` in `fetch`) is against an already-resolved, no-symlinks-no-double-slashes path. Without this, a `fetch("/root/../etc/passwd")` could escape the root through a symlink that was resolved after the `starts_with` check.

### Default extensions

`md`, `markdown`, `txt`, `rst`, `csv`, `json`, `yaml`, `yml`, `rs`, `py`, `js`, `ts`, `tsx`, `jsx`, `go`, `java`, `html`, `htm`, `log`, `toml`. Overridable via `with_extensions`. The default set is the intersection of "common textual formats developers expect to be searchable" and "formats whose bodies are actually UTF-8."

Binaries (`.bin`, `.png`, `.pdf`, etc.) are skipped at the extension filter before any I/O — even attempting `read_to_string` on them would produce garbage or errors.

### Size cap

`max_file_bytes` defaults to 2 MiB. Files larger than that are skipped with a `CfgError::Validation` that the runtime logs as a warning. Two reasons:

1. **Embedding cost.** A 50 MiB log file produces thousands of chunks and thousands of embedder calls. That's a budgeting decision the operator should make deliberately.
2. **Chunker quality.** The semantic chunker's heuristics assume prose-like inputs. A 20 MiB minified JSON blob will produce garbage chunks.

### Watch mode

With the default `watch` feature enabled, `subscribe()` returns a stream backed by `notify`. Implementation details worth flagging:

- **Dedicated OS thread.** `notify::recommended_watcher` calls into platform APIs (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows) that are not tokio-friendly. We run the watcher on a dedicated `std::thread` and forward events via `tokio::sync::mpsc::unbounded_channel`.
- **Inline `ReceiverStream`.** Converting a tokio `UnboundedReceiver` to a futures `Stream` is normally done by `tokio-stream::wrappers::UnboundedReceiverStream`. I inlined a five-line equivalent rather than take a new dependency for one struct.
- **Event translation.** `Create` and `Modify` produce `Upsert` events (the connector re-reads the file and emits a fresh `Resource`). `Remove` produces `Delete` with the path as the id. `Access` and pure-metadata changes are ignored.

### What's not in the connector

- **`.gitignore` / `.cfgignore` support.** Typical repos include build directories, node_modules, generated files. Ignore-pattern support is the right fix; currently users must scope the root narrowly or override the extension list.
- **Symlink handling.** We `follow_links(false)` to stay inside the root, which means a symlink to external content is silently ignored. Might want an explicit "follow symlinks but only within root" mode.
- **Large-directory streaming.** `WalkDir` collects eagerly. For a 100k-file root this is fine (each entry is ~200 bytes). For a 10M-file root it's a problem. Switch to a streaming walker at that scale.

---

## 5. Running the acceptance test

The scaffolding brief's Phase 12 end-to-end acceptance test becomes runnable once this drop-in lands:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn scaffold_end_to_end() -> anyhow::Result<()> {
    // 1. Stand up DB + vector store.
    let env = cfg_test::Env::spin_up().await?;

    // 2. Register the compiled schema with the engine.
    env.apply_schema("examples/engineering-context/schemas/doc.csl").await?;

    // 3. Bind a local-files connector to the schema.
    let connector = Arc::new(
        cfg_connector_local_files::LocalFilesConnector::new(
            "examples/engineering-context/sample-docs",
        )?,
    );
    env.ingest_runtime().register(ConnectorBinding {
        connector,
        target_namespace: "examples".into(),
        target_schema:    "Doc".into(),
        interval: None,
    });

    // 4. One-shot sync.
    let outcome = env.ingest_runtime().sync_once("local-files/examples.Doc").await;
    assert_eq!(outcome.kind, SyncOutcomeKind::Completed);
    assert!(outcome.resources_ingested > 0);

    // 5. Query.
    let r = env.query("postgres setup").await?;
    assert!(!r.results.is_empty());
    Ok(())
}
```

This is the first test in the whole stack that exercises every layer — CSL → migrations → ingestor → connector → retriever → policy → receipts — against real data. When it goes green on CI, v0.1 of the platform is feature-complete for the scaffolding acceptance criterion.

---

## 6. Known gaps — priority order

1. **Incremental sync.** Write connector state (`last_sync_at`, `cursor`) to `cfg_ingest_state` on every run. Skip unchanged resources on the next run. Significant efficiency win at scale.

2. **Delete propagation.** `ChangeEvent::Delete` is parsed but not acted on — the ingest runtime logs it and moves on. Needs a `(connector_resource_id, engine_record_id)` lookup in `cfg_lineage` so we can delete the right row.

3. **GitHub connector.** `connectors/github/` is the next reference implementation. OAuth device flow, rate-limit handling, issue + PR + markdown-in-repo ingestion. Stage for that is the largest step toward proving the connector SDK scales beyond trivial cases.

4. **Slack connector.** `connectors/slack/` after GitHub. Bot token auth, channel allow-list, chunking strategy that respects message threads.

5. **Dead-letter queue.** `cfg_ingest_dlq` table with `(connector_id, resource_id, last_error, attempts, next_retry_at)`. CLI command to replay.

6. **Rate limiting / backoff.** Vendor APIs return `429` with `Retry-After` headers. Connectors should expose an idiomatic retry strategy rather than letting every author reinvent one.

7. **Parallel ingestion.** Current runtime ingests resources serially. `list()` is a stream; we can run N embeddings in parallel via `buffer_unordered`. Careful with embedder rate limits; expose concurrency as a per-binding config.

8. **`.gitignore` support for `local-files`.** Two-line change: add `ignore` crate (the one `ripgrep` uses) and check each path.

9. **Binary file handling.** PDFs and Office docs are common in enterprise. A `cfg-extract` crate with `pdf-extract` and `docx-rs` support, invoked from the connector before returning the `Resource`.

10. **Connector healthcheck surface.** `POST /v1/connectors/:id/healthcheck` wires up but isn't implemented on the REST side yet. Low-hanging once the binding is accessible through `AppState`.

---

## 7. Verification checklist

Things most likely to need adjustment:

1. **`notify` 7 API.** `notify::recommended_watcher` signature has evolved across majors; the closure parameter type is `Result<Event, Error>`. If it's `std::result::Result` vs `notify::Result`, one import fix.

2. **`walkdir::DirEntry::into_path()`.** Exists in 2.x. If the compiler complains, use `entry.path().to_owned()` instead.

3. **`tempfile` crate in dev-deps.** I used it in the `local-files` tests. Already in the workspace scaffolding brief; should resolve.

4. **`time::OffsetDateTime::from_unix_timestamp`.** Takes `i64`, returns `Result<_, _>`. My code unwraps with a fallback to `now_utc()` — that should be fine.

5. **`async_trait` on default methods.** `Connector::subscribe` and `::healthcheck` have default impls. `#[async_trait]` can be finicky about default impls + lifetime ellision on trait methods; if compilation complains, spell out `fn subscribe<'async_trait>(&'async_trait self) -> Pin<Box<dyn Future<Output = _> + Send + 'async_trait>>` explicitly. Unusual but occasionally needed.

6. **`path.display().to_string()`.** On Windows, this can produce surprising byte sequences for non-UTF-8 paths. `local-files` is documented as UTF-8-only; a lossier-but-stable encoding is future work.

The `cfg-connector-sdk` tests run without any external services. `cfg-ingest` unit tests are registry-only and also run clean. `local-files` tests need a writable `tempfile::tempdir` but nothing else. Smoke-test those three first.

*End of ingestion design notes.*
