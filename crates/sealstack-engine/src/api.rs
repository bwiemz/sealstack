//! Public API surface that `sealstack-gateway` and other external crates consume.
//!
//! This module is the single source of truth for the shapes crossing the
//! engine boundary. Keeping it isolated in a small module (no sqlx, no wasmtime,
//! no vector-store specifics) lets the gateway depend on [`EngineHandle`] without
//! pulling in the entire runtime at build time.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Caller — authenticated principal handed in by the gateway.
// ---------------------------------------------------------------------------

/// The authenticated principal making a request.
///
/// Produced by the gateway's auth middleware from a verified JWT; passed verbatim
/// into every engine method. Policy predicates reference its fields as `caller.*`.
///
/// Field shape mirrors `sealstack_gateway::mcp::types::Caller` exactly so callers can
/// `.into()`-convert one into the other.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Caller {
    /// Stable user identifier from the identity provider (`sub` claim).
    pub id: String,
    /// Tenant / workspace identifier. Empty string means the default tenant.
    #[serde(default)]
    pub tenant: String,
    /// Group memberships relevant for policy evaluation (e.g. `"eng"`, `"hr"`).
    #[serde(default)]
    pub groups: Vec<String>,
    /// Roles carried in the token (e.g. `"admin"`, `"viewer"`).
    #[serde(default)]
    pub roles: Vec<String>,
    /// Extra attribute map for custom JWT claims.
    #[serde(default)]
    pub attrs: serde_json::Map<String, Value>,
}

impl Caller {
    /// Construct a minimal caller for tests.
    #[must_use]
    pub fn test(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tenant: String::new(),
            groups: vec![],
            roles: vec![],
            attrs: serde_json::Map::new(),
        }
    }

    /// Returns true if the caller carries the given role.
    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// Returns true if the caller belongs to the given group.
    #[must_use]
    pub fn in_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }
}

// ---------------------------------------------------------------------------
// Error type.
// ---------------------------------------------------------------------------

/// Errors surfaced by [`EngineHandle`] methods.
///
/// The gateway converts these into MCP JSON-RPC error codes; see
/// `sealstack_gateway::mcp::protocol`.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The requested record does not exist.
    #[error("record not found")]
    NotFound,
    /// The caller is authenticated but the policy engine denied the action.
    #[error("policy denied: {reason}")]
    PolicyDenied {
        /// Human-readable reason; do not leak data through this string.
        reason: String,
    },
    /// The arguments to the call were malformed or semantically invalid.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// Backend (DB, vector store, embedder, network) failure.
    #[error("backend failure: {0}")]
    Backend(String),
    /// An unknown or unregistered schema was referenced.
    #[error("unknown schema `{namespace}.{schema}`")]
    UnknownSchema {
        /// CSL namespace.
        namespace: String,
        /// CSL schema name.
        schema: String,
    },
}

impl EngineError {
    /// Build a [`Backend`](Self::Backend) error from any `Display`able value.
    pub fn backend(e: impl std::fmt::Display) -> Self {
        Self::Backend(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Request / Response DTOs.
// ---------------------------------------------------------------------------

/// `search_<schema>` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    /// Authenticated caller.
    pub caller: Caller,
    /// Schema namespace.
    pub namespace: String,
    /// Schema name.
    pub schema: String,
    /// Natural-language query.
    pub query: String,
    /// Maximum hits to return (post-reranking).
    pub top_k: usize,
    /// Structured facet filters.
    pub filters: Value,
}

/// One search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Primary key of the matched record.
    pub id: String,
    /// Fused relevance score after retrieval + freshness + reranking.
    pub score: f32,
    /// Snippet of text likely to have matched. Source: the `@chunked` field.
    pub excerpt: String,
    /// The record itself as a JSON object.
    pub record: Value,
}

/// `search_<schema>` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Receipt ID clients can later fetch via `/v1/receipts/:id`.
    pub receipt_id: String,
    /// Ranked hits.
    pub results: Vec<SearchHit>,
}

/// `get_<schema>` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRequest {
    /// Authenticated caller.
    pub caller: Caller,
    /// Schema namespace.
    pub namespace: String,
    /// Schema name.
    pub schema: String,
    /// Primary key.
    pub id: String,
}

/// `list_<schema>` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
    /// Authenticated caller.
    pub caller: Caller,
    /// Schema namespace.
    pub namespace: String,
    /// Schema name.
    pub schema: String,
    /// Facet filters as JSON object; keys are facet field names.
    pub filters: Value,
    /// Opaque cursor for continuation.
    pub cursor: Option<String>,
    /// Page size cap.
    pub limit: usize,
}

/// `list_<schema>_<relation>` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRelationRequest {
    /// Authenticated caller.
    pub caller: Caller,
    /// Schema namespace of the parent.
    pub namespace: String,
    /// Parent schema.
    pub schema: String,
    /// Name of the relation in CSL.
    pub relation: String,
    /// Primary key of the parent record.
    pub parent_id: String,
    /// Opaque cursor for continuation.
    pub cursor: Option<String>,
    /// Page size cap.
    pub limit: usize,
}

/// Response used by both `list_*` and `list_*_<relation>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    /// Items on this page.
    pub items: Vec<Value>,
    /// Cursor for the next page, or `None` when exhausted.
    pub next_cursor: Option<String>,
}

/// `aggregate_<schema>_<facet>` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateRequest {
    /// Authenticated caller.
    pub caller: Caller,
    /// Schema namespace.
    pub namespace: String,
    /// Schema name.
    pub schema: String,
    /// Facet column to aggregate on. Must be a `@facet` field in CSL.
    pub facet: String,
    /// Filters applied before aggregation.
    pub filters: Value,
    /// Maximum buckets to return.
    pub limit: usize,
}

/// One bucket in an aggregation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateBucket {
    /// The facet value.
    pub key: Value,
    /// Count of records with that value.
    pub count: u64,
}

/// Aggregation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateResponse {
    /// Buckets sorted by `count` descending.
    pub buckets: Vec<AggregateBucket>,
}

// ---------------------------------------------------------------------------
// The trait.
// ---------------------------------------------------------------------------

/// The engine's external interface.
///
/// Implemented by [`crate::Engine`]. A `dyn EngineHandle` is held by the
/// gateway's `AppState`. All methods are cancel-safe and take `&self` so a
/// single `Arc<Engine>` services all concurrent requests.
#[async_trait]
pub trait EngineHandle: Send + Sync + 'static {
    /// Hybrid search (BM25 + vector) across a schema's `@chunked` and
    /// `@searchable` fields, with permissions applied pre-LLM.
    async fn search(&self, req: SearchRequest) -> Result<SearchResponse, EngineError>;

    /// Fetch one record by primary key.
    ///
    /// Returns [`EngineError::NotFound`] if the record is missing or if policy
    /// hides its existence.
    async fn get(&self, req: GetRequest) -> Result<Value, EngineError>;

    /// Paged list of records, optionally facet-filtered.
    async fn list(&self, req: ListRequest) -> Result<ListResponse, EngineError>;

    /// List records on the far side of a named relation.
    async fn list_relation(&self, req: ListRelationRequest) -> Result<ListResponse, EngineError>;

    /// Grouped aggregate over one facet.
    async fn aggregate(&self, req: AggregateRequest) -> Result<AggregateResponse, EngineError>;
}
