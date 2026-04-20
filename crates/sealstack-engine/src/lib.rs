//! SealStack engine — the retrieval, ingestion, and policy runtime.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                      sealstack_gateway                              │
//! │  (MCP tool handlers call through to `dyn api::EngineHandle`)  │
//! └───────────────────────────┬──────────────────────────────────┘
//!                             │ async calls
//! ┌───────────────────────────▼──────────────────────────────────┐
//! │                     sealstack_engine::Engine                       │
//! │   ┌──────────────┬──────────────┬──────────────┐            │
//! │   │ SchemaReg    │ Retrieval    │ Policy       │            │
//! │   │              │  ┌─────────┐ │              │            │
//! │   │              │  │  BM25   │ │              │            │
//! │   │              │  │  Vector │ │              │            │
//! │   │              │  │ Rerank  │ │              │            │
//! │   │              │  └─────────┘ │              │            │
//! │   ├──────────────┼──────────────┼──────────────┤            │
//! │   │ Ingest       │ Receipts     │ Freshness    │            │
//! │   └──────────────┴──────────────┴──────────────┘            │
//! └───────┬────────────────────────────┬────────────────────────┘
//!         │                            │
//! ┌───────▼──────────┐     ┌──────────▼───────────┐
//! │   sealstack_vectorstore │     │ sqlx::PgPool         │
//! │   (trait, impl)   │     │ (Postgres 16)        │
//! └───────────────────┘     └──────────────────────┘
//! ```
//!
//! ## Ownership model
//!
//! A single [`Engine`] instance is created once at process boot and shared across
//! all Tokio tasks via `Arc<Engine>`. The gateway holds `Arc<dyn api::EngineHandle>`
//! which dispatches to the same instance; nothing about the dispatch path allocates
//! per-request.
//!
//! ## Crate layout
//!
//! | Module              | Purpose                                                  |
//! |---------------------|----------------------------------------------------------|
//! | [`api`]             | Trait the gateway depends on + request/response types    |
//! | [`config`]          | Engine configuration                                     |
//! | [`engine`]          | The `Engine` struct — holds every subsystem              |
//! | [`schema_registry`] | In-memory registry of compiled CSL schemas               |
//! | [`store`]           | Postgres pool + migration runner                         |
//! | [`ingest`]          | Connector → typed rows + chunks + vectors                |
//! | [`retrieval`]       | Hybrid BM25 + vector search + reranking                  |
//! | [`policy`]          | Policy predicate evaluation (WASM-bound; stub in v0.1)   |
//! | [`receipts`]        | Receipt creation and persistence                         |
//! | [`freshness`]       | Freshness decay score functions                          |
//! | [`rerank`]          | Reranker abstraction                                     |

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod api;
pub mod config;
pub mod engine;
pub mod facade;
pub mod freshness;
pub mod ingest;
pub mod policy;
pub mod receipts;
pub mod rerank;
pub mod retrieval;
pub mod schema_registry;
pub mod store;
pub(crate) mod util;

pub use api::{
    AggregateBucket, AggregateRequest, AggregateResponse, Caller, EngineError, EngineHandle,
    GetRequest, ListRelationRequest, ListRequest, ListResponse, SearchHit, SearchRequest,
    SearchResponse,
};
pub use config::EngineConfig;
pub use engine::Engine;
pub use facade::EngineFacade;
