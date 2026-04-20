//! Grounded-answer receipts: caller, sources, policies, optional signature.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub id: ulid::Ulid,
    pub caller: String,
    pub query: String,
    pub sources: Vec<SourceRef>,
    pub policies: Vec<PolicyDecision>,
    pub answer_hash: Option<Vec<u8>>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: time::OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub chunk_id: ulid::Ulid,
    pub source_uri: String,
    pub score: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub rule: String,
    pub outcome: String,
}
