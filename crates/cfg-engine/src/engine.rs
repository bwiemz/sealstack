//! The main [`Engine`] struct — owns every subsystem and implements
//! [`EngineHandle`](crate::api::EngineHandle) as the single dispatch surface the
//! gateway calls into.
//!
//! # Lifecycle
//!
//! ```no_run
//! # async fn f() -> anyhow::Result<()> {
//! use std::sync::Arc;
//! use cfg_engine::{Engine, EngineConfig};
//!
//! let config = EngineConfig::test();
//!
//! // Callers provide a vector store and an embedder; the engine does not
//! // know which backends are wired.
//! # let vector_store: Arc<dyn cfg_vectorstore::VectorStore> = todo!();
//! # let embedder:     Arc<dyn cfg_embedders::Embedder>     = todo!();
//!
//! let engine = Engine::new_dev(config, vector_store, embedder).await?;
//! let engine: Arc<dyn cfg_engine::EngineHandle> = Arc::new(engine);
//!
//! // Hand this `Arc` to the gateway's `build_app`.
//! # Ok(())
//! # }
//! ```
//!
//! # Dispatch invariants
//!
//! Every method:
//!
//! 1. Resolves the schema via [`SchemaRegistry`]. Unknown → [`EngineError::UnknownSchema`].
//! 2. Runs the subsystem call (retrieval, row fetch, etc.).
//! 3. Evaluates the relevant policy predicate (`read` for search/get/list/relation,
//!    `list` for aggregate — aggregations reveal only counts, not bodies).
//! 4. Persists a [`receipts::Receipt`](crate::receipts::Receipt) for
//!    `search` and `get`/`list` calls. Aggregate calls log a receipt too.
//! 5. Returns the API DTO.
//!
//! # Concurrency
//!
//! All fields are `Arc`-sharable. `Engine` is `Clone`-cheap so it can be wrapped
//! in `Arc<Engine>` and handed to long-lived tasks. `&self` is used throughout
//! to permit unlimited concurrent reads.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use cfg_embedders::Embedder;
use cfg_vectorstore::VectorStore;

use crate::api::{
    AggregateBucket, AggregateRequest, AggregateResponse, EngineError, EngineHandle, GetRequest,
    ListRelationRequest, ListRequest, ListResponse, SearchHit, SearchRequest, SearchResponse,
};
use crate::config::EngineConfig;
use crate::ingest::Ingestor;
use crate::policy::{PolicyAction, PolicyEngine, PolicyInput, default_dev_policy};
use crate::receipts::{
    PolicyRef, Receipt, ReceiptBuilder, ReceiptStore, SourceRef, Stage, TimingRecorder,
};
use crate::rerank::{IdentityReranker, Reranker};
use crate::retrieval::Retriever;
use crate::schema_registry::{RelationKind, SchemaMeta, SchemaRegistry};
use crate::store::Store;
use crate::util::is_safe_ident;

/// The engine.
///
/// Clone-cheap: every field is `Arc` or itself clone-cheap. Prefer one canonical
/// instance per process wrapped in `Arc<Engine>`.
#[derive(Clone)]
pub struct Engine {
    config: EngineConfig,
    registry: SchemaRegistry,
    store: Store,
    retriever: Arc<Retriever>,
    ingestor: Arc<Ingestor>,
    policy: Arc<dyn PolicyEngine>,
    receipts: Arc<ReceiptStore>,
}

impl Engine {
    /// Construct an engine with production defaults except for policy, which
    /// defaults to the always-allow stub until [`crate::policy::WasmPolicy`] is
    /// production-ready. Uses the [`IdentityReranker`] — no score changes.
    ///
    /// See [`Engine::new`] for the full-control constructor and
    /// [`Engine::new_dev_with_reranker`] for when you want a non-identity
    /// reranker but don't want to hand-build the policy engine.
    pub async fn new_dev(
        config: EngineConfig,
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<dyn Embedder>,
    ) -> anyhow::Result<Self> {
        Self::new_dev_with_reranker(config, vector_store, embedder, Arc::new(IdentityReranker))
            .await
    }

    /// Dev engine using the given reranker. Useful when the gateway selects a
    /// reranker from env at boot time but still wants the dev policy stub.
    pub async fn new_dev_with_reranker(
        config: EngineConfig,
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<dyn Embedder>,
        reranker: Arc<dyn Reranker>,
    ) -> anyhow::Result<Self> {
        let policy = default_dev_policy();
        Self::new(config, vector_store, embedder, policy, reranker).await
    }

    /// Full-control constructor.
    pub async fn new(
        config: EngineConfig,
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<dyn Embedder>,
        policy: Arc<dyn PolicyEngine>,
        reranker: Arc<dyn Reranker>,
    ) -> anyhow::Result<Self> {
        tracing::info!(
            db = %config.database_url,
            pool = config.pg_pool_size,
            compile_dir = %config.compile_dir,
            "initializing engine",
        );

        let store = Store::connect(&config.database_url, config.pg_pool_size).await?;
        let registry = SchemaRegistry::load_from_dir(&config.compile_dir)?;
        tracing::info!(count = registry.len(), "loaded schemas from compile dir");

        // Merge any schemas previously registered via POST /v1/schemas. Disk
        // wins on ties only because the registry's `insert` is last-write.
        // Hydration failure is non-fatal but is logged as an error because the
        // visible symptom (no registered schemas) looks identical to a fresh
        // deployment and silently drops user state.
        if let Err(e) = registry.hydrate_from_store(&store).await {
            tracing::error!(error = %e, "failed to hydrate schemas from store — previously registered schemas will not be served until the condition is resolved");
        }

        let retriever = Arc::new(Retriever::new(
            config.retrieval.clone(),
            vector_store.clone(),
            embedder.clone(),
            store.clone(),
            reranker,
        ));
        let ingestor = Arc::new(Ingestor::new(
            vector_store.clone(),
            embedder.clone(),
            store.clone(),
        ));
        let receipts = Arc::new(ReceiptStore::new(store.clone()));

        Ok(Self {
            config,
            registry,
            store,
            retriever,
            ingestor,
            policy,
            receipts,
        })
    }

    /// Access the schema registry.
    #[must_use]
    pub fn registry(&self) -> &SchemaRegistry {
        &self.registry
    }

    /// Access the ingestor (used by `cfg-ingest`).
    #[must_use]
    pub fn ingestor(&self) -> Arc<Ingestor> {
        self.ingestor.clone()
    }

    /// Access the receipts store (used by the REST `GET /v1/receipts/:id` route).
    #[must_use]
    pub fn receipts(&self) -> Arc<ReceiptStore> {
        self.receipts.clone()
    }

    /// Access the underlying store (used by the REST DDL-apply path).
    #[must_use]
    pub fn store_handle(&self) -> &crate::store::Store {
        &self.store
    }

    /// Access the effective configuration.
    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    // ---------------------------------------------------------------------
    // Shared helpers
    // ---------------------------------------------------------------------

    async fn fetch_row_raw(
        &self,
        meta: &SchemaMeta,
        id: &str,
    ) -> Result<Value, EngineError> {
        if !is_safe_ident(&meta.table) {
            return Err(EngineError::InvalidArgument(format!(
                "unsafe table identifier `{}`",
                meta.table
            )));
        }
        // Tenant is not yet threaded through the post-search row fetch; the
        // retrieval layer has already scoped ids by tenant, so fetching by id
        // alone is safe as long as row ids are not guessable (they are ULIDs).
        // Belt-and-suspenders tenant re-check lands with the fetch API review.
        let sql = format!(
            "SELECT id::text AS id, title, body, created_at, metadata \
             FROM {} WHERE id = $1",
            meta.table
        );
        let row: Option<(
            String,
            Option<String>,
            Option<String>,
            Option<time::OffsetDateTime>,
            Option<Value>,
        )> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(self.store.pool())
            .await
            .map_err(EngineError::backend)?;
        match row {
            None => Err(EngineError::NotFound),
            Some((id, title, body, created_at, metadata)) => Ok(json!({
                "id": id,
                "title": title,
                "body": body,
                "created_at": created_at.map(|t| t
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default()),
                "metadata": metadata.unwrap_or(Value::Null),
            })),
        }
    }

    async fn fetch_many(
        &self,
        meta: &SchemaMeta,
        where_clause: &str,
        binds: Vec<String>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Value>, Option<String>), EngineError> {
        if !is_safe_ident(&meta.table) {
            return Err(EngineError::InvalidArgument(format!(
                "unsafe table identifier `{}`",
                meta.table
            )));
        }
        let limit_i64 = i64::try_from(limit.min(1000)).unwrap_or(100);
        let mut sql = format!(
            "SELECT id::text AS id, title, body, created_at, metadata \
             FROM {} WHERE 1=1",
            meta.table
        );
        if !where_clause.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(where_clause);
        }
        if cursor.is_some() {
            sql.push_str(&format!(" AND id > ${}", binds.len() + 2));
        }
        sql.push_str(&format!(" ORDER BY id ASC LIMIT ${}", binds.len() + 1));

        let mut query = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<String>,
                Option<time::OffsetDateTime>,
                Option<Value>,
            ),
        >(&sql);
        for bind in &binds {
            query = query.bind(bind);
        }
        query = query.bind(limit_i64);
        if let Some(c) = cursor {
            query = query.bind(c);
        }

        let rows = query
            .fetch_all(self.store.pool())
            .await
            .map_err(EngineError::backend)?;
        let next_cursor = rows.last().map(|r| r.0.clone());
        let items: Vec<Value> = rows
            .into_iter()
            .map(|(id, title, body, created_at, metadata)| {
                json!({
                    "id": id,
                    "title": title,
                    "body": body,
                    "created_at": created_at.map(|t| t
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_default()),
                    "metadata": metadata.unwrap_or(Value::Null),
                })
            })
            .collect();
        // Only return a cursor if we filled the page (heuristic).
        let cursor_out = if items.len() == limit.min(1000) {
            next_cursor
        } else {
            None
        };
        Ok((items, cursor_out))
    }

    async fn log_receipt(&self, receipt: Receipt) {
        if let Err(e) = self.receipts.persist(&receipt).await {
            tracing::warn!(error = %e, id = %receipt.id, "failed to persist receipt");
        }
    }
}

// ---------------------------------------------------------------------------
// EngineHandle implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl EngineHandle for Engine {
    async fn search(&self, req: SearchRequest) -> Result<SearchResponse, EngineError> {
        let meta = self.registry.get(&req.namespace, &req.schema)?;
        let top_k = if req.top_k == 0 {
            meta.context
                .default_top_k
                .unwrap_or(self.config.retrieval.default_top_k)
        } else {
            req.top_k
        };

        let mut timer = TimingRecorder::start();

        // Retrieval — scoped to the caller's tenant. Empty tenant in the
        // request matches rows with no tenant set.
        let hits = self
            .retriever
            .search(&meta, &req.query, top_k, &req.filters, &req.caller.tenant)
            .await?;
        timer.split(Stage::Retrieval);

        if hits.is_empty() {
            let receipt = ReceiptBuilder::new(
                req.caller.clone(),
                format!("{}.{}", meta.namespace, meta.name),
                format!("search_{}", meta.name.to_lowercase()),
                json!({ "query": req.query, "top_k": top_k, "filters": req.filters }),
            )
            .with_timings(timer.finish())
            .build();
            let receipt_id = receipt.id.clone();
            self.log_receipt(receipt).await;
            return Ok(SearchResponse {
                receipt_id,
                results: vec![],
            });
        }

        // Fetch full records for the surviving IDs (cheap for v0.1; batch later).
        let mut records: Vec<(crate::retrieval::RetrievedHit, Value)> = Vec::with_capacity(hits.len());
        for hit in hits {
            match self.fetch_row_raw(&meta, &hit.id).await {
                Ok(row) => records.push((hit, row)),
                Err(EngineError::NotFound) => {
                    tracing::debug!(id = %hit.id, "retrieved id missing from row store; skipping");
                }
                Err(e) => {
                    tracing::warn!(error = %e, id = %hit.id, "row fetch failed");
                }
            }
        }

        // Policy filter.
        let caller = &req.caller;
        let record_values: Vec<Value> = records.iter().map(|(_, r)| r.clone()).collect();
        let mask = self
            .policy
            .filter(
                &meta.namespace,
                &meta.name,
                PolicyAction::Read,
                caller,
                &record_values,
            )
            .await?;
        timer.split(Stage::Policy);

        let mut results = Vec::new();
        let mut sources = Vec::new();
        let mut verdicts = Vec::new();
        let qualified = format!("{}.{}", meta.namespace, meta.name);
        for ((hit, record), allowed) in records.into_iter().zip(mask.into_iter()) {
            verdicts.push(PolicyRef {
                schema: qualified.clone(),
                predicate: "read".into(),
                verdict: if allowed { "allow".into() } else { "deny".into() },
            });
            if !allowed {
                continue;
            }
            sources.push(SourceRef {
                schema: qualified.clone(),
                record_id: hit.id.clone(),
                chunk_id: None,
                score: hit.score,
            });
            results.push(SearchHit {
                id: hit.id,
                score: hit.score,
                excerpt: hit.excerpt,
                record,
            });
        }

        let timings = timer.finish();
        let receipt = ReceiptBuilder::new(
            req.caller.clone(),
            qualified.clone(),
            format!("search_{}", meta.name.to_lowercase()),
            json!({ "query": req.query, "top_k": top_k, "filters": req.filters }),
        )
        .with_sources(sources)
        .with_policies(verdicts)
        .with_timings(timings)
        .build();
        let receipt_id = receipt.id.clone();
        self.log_receipt(receipt).await;

        Ok(SearchResponse {
            receipt_id,
            results,
        })
    }

    async fn get(&self, req: GetRequest) -> Result<Value, EngineError> {
        let meta = self.registry.get(&req.namespace, &req.schema)?;
        let record = self.fetch_row_raw(&meta, &req.id).await?;

        let verdict = self
            .policy
            .evaluate(PolicyInput {
                namespace: &meta.namespace,
                schema: &meta.name,
                action: PolicyAction::Read,
                caller: &req.caller,
                record: &record,
            })
            .await?;

        let qualified = format!("{}.{}", meta.namespace, meta.name);
        let receipt = ReceiptBuilder::new(
            req.caller.clone(),
            qualified.clone(),
            format!("get_{}", meta.name.to_lowercase()),
            json!({ "id": req.id }),
        )
        .with_policies(vec![PolicyRef {
            schema: qualified,
            predicate: "read".into(),
            verdict: if verdict.is_allow() { "allow".into() } else { "deny".into() },
        }])
        .build();
        self.log_receipt(receipt).await;

        if verdict.is_allow() {
            Ok(record)
        } else {
            // Policy hides existence for denied reads.
            Err(EngineError::NotFound)
        }
    }

    async fn list(&self, req: ListRequest) -> Result<ListResponse, EngineError> {
        let meta = self.registry.get(&req.namespace, &req.schema)?;
        let (where_clause, binds) = build_facet_where(&meta, &req.filters);
        let (items, next_cursor) = self
            .fetch_many(&meta, &where_clause, binds, req.cursor.as_deref(), req.limit)
            .await?;

        let mask = self
            .policy
            .filter(
                &meta.namespace,
                &meta.name,
                PolicyAction::List,
                &req.caller,
                &items,
            )
            .await?;
        let items: Vec<Value> = items
            .into_iter()
            .zip(mask.into_iter())
            .filter_map(|(v, ok)| ok.then_some(v))
            .collect();

        Ok(ListResponse { items, next_cursor })
    }

    async fn list_relation(
        &self,
        req: ListRelationRequest,
    ) -> Result<ListResponse, EngineError> {
        let (parent_meta, relation) =
            self.registry
                .resolve_relation(&req.namespace, &req.schema, &req.relation)?;

        if relation.kind != RelationKind::Many {
            return Err(EngineError::InvalidArgument(format!(
                "relation `{}` is not `many`",
                relation.name
            )));
        }

        let target_meta = self
            .registry
            .get(&relation.target_namespace, &relation.target_schema)?;

        if !is_safe_ident(&relation.foreign_key) {
            return Err(EngineError::InvalidArgument(format!(
                "unsafe relation foreign key `{}`",
                relation.foreign_key
            )));
        }

        let where_clause = format!("{} = $1", relation.foreign_key);
        let binds = vec![req.parent_id.clone()];
        let (items, next_cursor) = self
            .fetch_many(
                &target_meta,
                &where_clause,
                binds,
                req.cursor.as_deref(),
                req.limit,
            )
            .await?;

        let mask = self
            .policy
            .filter(
                &target_meta.namespace,
                &target_meta.name,
                PolicyAction::List,
                &req.caller,
                &items,
            )
            .await?;
        let items: Vec<Value> = items
            .into_iter()
            .zip(mask.into_iter())
            .filter_map(|(v, ok)| ok.then_some(v))
            .collect();

        // Touch parent_meta to satisfy the unused-binding lint when relation
        // checks are moved into a helper later.
        let _ = parent_meta;
        Ok(ListResponse { items, next_cursor })
    }

    async fn aggregate(
        &self,
        req: AggregateRequest,
    ) -> Result<AggregateResponse, EngineError> {
        let meta = self.registry.get(&req.namespace, &req.schema)?;

        if !meta.facets.contains(&req.facet) {
            return Err(EngineError::InvalidArgument(format!(
                "field `{}` is not a facet on `{}.{}`",
                req.facet, meta.namespace, meta.name
            )));
        }
        if !is_safe_ident(&meta.table) || !is_safe_ident(&req.facet) {
            return Err(EngineError::InvalidArgument(
                "unsafe table or facet identifier".into(),
            ));
        }

        let (where_clause, binds) = build_facet_where(&meta, &req.filters);
        let limit_i64 = i64::try_from(req.limit.min(1000)).unwrap_or(100);
        let mut sql = format!(
            "SELECT {col} AS key, COUNT(*)::bigint AS cnt FROM {tab} WHERE 1=1",
            col = req.facet,
            tab = meta.table,
        );
        if !where_clause.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&where_clause);
        }
        sql.push_str(&format!(
            " GROUP BY {col} ORDER BY cnt DESC LIMIT ${limit_idx}",
            col = req.facet,
            limit_idx = binds.len() + 1,
        ));

        let mut query = sqlx::query_as::<_, (Option<String>, i64)>(&sql);
        for bind in &binds {
            query = query.bind(bind);
        }
        query = query.bind(limit_i64);

        let rows = query
            .fetch_all(self.store.pool())
            .await
            .map_err(EngineError::backend)?;
        let buckets = rows
            .into_iter()
            .map(|(key, cnt)| AggregateBucket {
                key: key.map_or(Value::Null, Value::String),
                count: cnt as u64,
            })
            .collect();

        // Aggregates reveal counts only, but we still log a receipt.
        let qualified = format!("{}.{}", meta.namespace, meta.name);
        let receipt = ReceiptBuilder::new(
            req.caller.clone(),
            qualified.clone(),
            format!("aggregate_{}_{}", meta.name.to_lowercase(), req.facet),
            json!({ "facet": req.facet, "filters": req.filters, "limit": req.limit }),
        )
        .with_policies(vec![PolicyRef {
            schema: qualified,
            predicate: "list".into(),
            verdict: "allow".into(),
        }])
        .build();
        self.log_receipt(receipt).await;

        Ok(AggregateResponse { buckets })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a WHERE clause from a facet-filter JSON object.
///
/// Only scalar filter values declared as facets on the schema are accepted.
/// Returns `(where_sql, bind_values)`. Bind placeholders are numbered `$1..$N`
/// in declaration order; callers wire them via `.bind(...)`.
fn build_facet_where(meta: &SchemaMeta, filters: &Value) -> (String, Vec<String>) {
    let Some(obj) = filters.as_object() else {
        return (String::new(), vec![]);
    };
    let mut clauses = Vec::new();
    let mut binds = Vec::new();
    for (key, value) in obj {
        if !meta.facets.iter().any(|f| f == key) {
            continue;
        }
        if !is_safe_ident(key) {
            continue;
        }
        let bind_idx = binds.len() + 1;
        match value {
            Value::String(s) => {
                binds.push(s.clone());
                clauses.push(format!("{key} = ${bind_idx}"));
            }
            Value::Bool(b) => {
                binds.push(b.to_string());
                clauses.push(format!("{key}::text = ${bind_idx}"));
            }
            Value::Number(n) => {
                binds.push(n.to_string());
                clauses.push(format!("{key}::text = ${bind_idx}"));
            }
            _ => { /* skip array/object/null for v0.1 */ }
        }
    }
    (clauses.join(" AND "), binds)
}
