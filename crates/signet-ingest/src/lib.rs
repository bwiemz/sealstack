//! Ingestion runtime.
//!
//! The runtime takes one or more registered connectors and drives them either
//! on demand (`sync_once`) or on a periodic schedule (`start_background`).
//! For each resource a connector yields, the runtime calls the engine's
//! [`Ingestor`](signet_engine::ingest::Ingestor) to persist the row and upsert
//! its chunks into the vector store.
//!
//! # Wiring overview
//!
//! ```text
//!                                      ┌──────────────────────┐
//!      signet connector sync <name>  ───▶ │   IngestRuntime      │
//!                                      │                      │
//!   signet connector start-background ──▶ │  sync_once / start   │
//!                                      │  cancellation        │
//!                                      │                      │
//!                                      └──────────┬───────────┘
//!                                                 │
//!                       ┌─────────────────────────┴──────────────────┐
//!                       │                                            │
//!              ┌────────▼───────┐                         ┌──────────▼──────┐
//!              │  Connector     │   ResourceStream        │   Engine        │
//!              │  (local-files, │ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ▶ │  ingestor()     │
//!              │   github, …)   │                         │  ingest()       │
//!              └────────────────┘                         └─────────────────┘
//! ```
//!
//! # v0.1 scope
//!
//! * One-shot `sync_once` that walks `connector.list()` and ingests every
//!   resource serially. No checkpointing; the full set is replayed each call.
//!   Safe because the ingestor's `upsert_row` is idempotent by primary key.
//! * Background poll loop with a configurable interval.
//! * Subscribe-based streaming when the connector provides a change stream.
//!
//! Incremental sync via the `signet_ingest_state` table is stubbed out and lands
//! in v0.2.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod registry;
pub mod runtime;

pub use registry::{ConnectorBinding, ConnectorRegistry};
pub use runtime::{IngestRuntime, SyncOutcome, SyncOutcomeKind};
