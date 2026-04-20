//! Receipt generation and persistence.
//!
//! Every tool call that surfaces data emits a receipt — an audit record
//! summarizing the caller, query, sources retrieved, policies applied, and
//! timings. Clients get the receipt ID back in the response and can fetch the
//! full record via `GET /v1/receipts/:id`.
//!
//! In Enterprise Edition, receipts are Ed25519-signed with the deployment's
//! service key and can be independently verified. v0.1 stores them unsigned.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use ulid::Ulid;

use crate::api::{Caller, EngineError};
use crate::store::Store;

/// A receipt describing one tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// Ulid-encoded receipt id.
    pub id: String,
    /// The calling principal.
    pub caller: Caller,
    /// Qualified schema name (e.g. `"acme.crm.Customer"`).
    pub qualified_schema: String,
    /// Tool name (e.g. `"search_customer"`).
    pub tool: String,
    /// Arguments as submitted.
    pub arguments: Value,
    /// Source records that contributed to the response.
    pub sources: Vec<SourceRef>,
    /// Policies evaluated and their verdicts.
    pub policies_applied: Vec<PolicyRef>,
    /// Per-stage timings in milliseconds.
    pub timings_ms: Timings,
    /// UTC creation time.
    pub created_at: OffsetDateTime,
}

/// One source contributing to a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    /// Qualified schema name.
    pub schema: String,
    /// Primary key of the source record.
    pub record_id: String,
    /// Optional chunk id when the source is a specific chunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    /// Score assigned at retrieval.
    pub score: f32,
}

/// One policy evaluation recorded on the receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRef {
    /// Qualified schema name.
    pub schema: String,
    /// Action (`"read"`, `"write"`, `"list"`, `"delete"`).
    pub predicate: String,
    /// `"allow"` or `"deny"`.
    pub verdict: String,
}

/// Per-stage latency breakdown.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timings {
    /// Time spent in embedding the query.
    pub embed: u32,
    /// Time spent in vector and BM25 retrieval.
    pub retrieval: u32,
    /// Time spent in the reranker.
    pub rerank: u32,
    /// Time spent evaluating policies.
    pub policy: u32,
    /// End-to-end wall time.
    pub total: u32,
}

/// Builder for collecting timings inline during a request.
pub struct TimingRecorder {
    start: Instant,
    stage_start: Instant,
    current: Timings,
}

impl TimingRecorder {
    /// Start recording now.
    #[must_use]
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            stage_start: now,
            current: Timings::default(),
        }
    }

    /// Record the stage that just completed and begin the next one.
    pub fn split(&mut self, stage: Stage) {
        let ms = self.stage_start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        match stage {
            Stage::Embed => self.current.embed = ms,
            Stage::Retrieval => self.current.retrieval = ms,
            Stage::Rerank => self.current.rerank = ms,
            Stage::Policy => self.current.policy = ms,
        }
        self.stage_start = Instant::now();
    }

    /// Finalize the recorder and return timings.
    #[must_use]
    pub fn finish(mut self) -> Timings {
        self.current.total = self.start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        self.current
    }
}

/// Stage identifier for [`TimingRecorder::split`].
#[derive(Clone, Copy)]
pub enum Stage {
    /// Query embedding.
    Embed,
    /// Retrieval (vector + BM25).
    Retrieval,
    /// Reranking.
    Rerank,
    /// Policy evaluation.
    Policy,
}

// ---------------------------------------------------------------------------
// Builder.
// ---------------------------------------------------------------------------

/// Fluent builder for [`Receipt`]s.
pub struct ReceiptBuilder {
    receipt: Receipt,
}

impl ReceiptBuilder {
    /// Start a new receipt for a tool call.
    pub fn new(
        caller: Caller,
        qualified_schema: impl Into<String>,
        tool: impl Into<String>,
        arguments: Value,
    ) -> Self {
        Self {
            receipt: Receipt {
                id: Ulid::new().to_string(),
                caller,
                qualified_schema: qualified_schema.into(),
                tool: tool.into(),
                arguments,
                sources: vec![],
                policies_applied: vec![],
                timings_ms: Timings::default(),
                created_at: OffsetDateTime::now_utc(),
            },
        }
    }

    /// Append sources.
    #[must_use]
    pub fn with_sources(mut self, sources: Vec<SourceRef>) -> Self {
        self.receipt.sources = sources;
        self
    }

    /// Append policy verdicts.
    #[must_use]
    pub fn with_policies(mut self, policies: Vec<PolicyRef>) -> Self {
        self.receipt.policies_applied = policies;
        self
    }

    /// Attach timings.
    #[must_use]
    pub fn with_timings(mut self, timings: Timings) -> Self {
        self.receipt.timings_ms = timings;
        self
    }

    /// Consume the builder and return the built receipt.
    #[must_use]
    pub fn build(self) -> Receipt {
        self.receipt
    }
}

// ---------------------------------------------------------------------------
// Persistence.
// ---------------------------------------------------------------------------

/// Persistence for receipts.
pub struct ReceiptStore {
    store: Store,
}

impl ReceiptStore {
    /// Create a new persistence layer over the engine's Postgres pool.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Persist a receipt. Returns its id on success.
    pub async fn persist(&self, receipt: &Receipt) -> Result<String, EngineError> {
        let body = serde_json::to_value(receipt).map_err(EngineError::backend)?;
        sqlx::query(
            "INSERT INTO signet_receipts (id, caller_id, qualified_schema, tool, body, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&receipt.id)
        .bind(&receipt.caller.id)
        .bind(&receipt.qualified_schema)
        .bind(&receipt.tool)
        .bind(body)
        .bind(receipt.created_at)
        .execute(self.store.pool())
        .await
        .map_err(EngineError::backend)?;
        Ok(receipt.id.clone())
    }

    /// Fetch a receipt by id.
    pub async fn fetch(&self, id: &str) -> Result<Receipt, EngineError> {
        let row: Option<(Value,)> =
            sqlx::query_as("SELECT body FROM signet_receipts WHERE id = $1")
                .bind(id)
                .fetch_optional(self.store.pool())
                .await
                .map_err(EngineError::backend)?;
        let (body,) = row.ok_or(EngineError::NotFound)?;
        serde_json::from_value(body).map_err(EngineError::backend)
    }

    /// Delete receipts older than `retention_days` days.
    pub async fn prune(&self, retention_days: u32) -> Result<u64, EngineError> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::days(i64::from(retention_days));
        let res = sqlx::query("DELETE FROM signet_receipts WHERE created_at < $1")
            .bind(cutoff)
            .execute(self.store.pool())
            .await
            .map_err(EngineError::backend)?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_assembles_a_valid_receipt() {
        let caller = Caller::test("u_42");
        let r = ReceiptBuilder::new(caller, "acme.crm.Customer", "search_customer",
            serde_json::json!({ "query": "acme" }))
            .with_sources(vec![SourceRef {
                schema: "acme.crm.Customer".into(),
                record_id: "c_1".into(),
                chunk_id: Some("k_1".into()),
                score: 0.8,
            }])
            .with_policies(vec![PolicyRef {
                schema: "acme.crm.Customer".into(),
                predicate: "read".into(),
                verdict: "allow".into(),
            }])
            .with_timings(Timings { embed: 5, retrieval: 30, rerank: 10, policy: 2, total: 50 })
            .build();
        assert!(!r.id.is_empty());
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.policies_applied.len(), 1);
        assert_eq!(r.timings_ms.total, 50);
    }
}
