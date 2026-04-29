//! Qdrant backend.
//!
//! Uses [`qdrant-client`] 1.12 against a running Qdrant instance at the
//! configured gRPC URL (typically `http://localhost:6334` for dev). The store
//! is stateless — every request spawns a new client call; connection pooling
//! is delegated to the underlying `tonic` channel.
//!
//! # Filters
//!
//! We translate a narrow JSON filter subset into Qdrant's native `Filter`:
//!
//! * `{ "field": "value" }` → `must: [FieldCondition(field matches "value")]`
//! * `{ "field": 42 }`      → `must: [FieldCondition(field matches 42)]`
//! * `{ "field": true }`    → `must: [FieldCondition(field matches true)]`
//! * `{ "field": [a, b] }`  → `must: [FieldCondition(field matches any of [a, b])]`
//!
//! Anything more complex falls back to a best-effort `must` chain. This covers
//! the filter shapes our CSL `@facet` fields produce; richer filters need an
//! explicit DSL upstream.

use std::collections::HashMap;

use async_trait::async_trait;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, DeletePointsBuilder, Distance as QDistance, Filter,
    PointId, PointStruct, PointsIdsList, SearchPointsBuilder, UpsertPointsBuilder, Value as QValue,
    VectorParamsBuilder, point_id::PointIdOptions, value::Kind as QKind,
};
use serde_json::Value;

use crate::{Chunk, Distance, SealStackError, SealStackResult, SearchResult, VectorStore};

/// Qdrant-backed vector store.
pub struct QdrantStore {
    client: Qdrant,
}

impl QdrantStore {
    /// Connect to a Qdrant instance.
    ///
    /// `url` is the gRPC endpoint, e.g. `http://localhost:6334`. An optional
    /// API key is picked up from the `QDRANT_API_KEY` environment variable.
    pub async fn connect(url: &str) -> SealStackResult<Self> {
        let mut builder = Qdrant::from_url(url);
        if let Ok(key) = std::env::var("QDRANT_API_KEY") {
            builder = builder.api_key(key);
        }
        let client = builder.build().map_err(SealStackError::backend)?;
        // Sanity-check: list collections at boot to fail fast on bad URLs.
        client
            .list_collections()
            .await
            .map_err(SealStackError::backend)?;
        tracing::info!(url, "qdrant connected");
        Ok(Self { client })
    }

    /// Access the underlying client (escape hatch for engine-level tools).
    #[must_use]
    pub fn client(&self) -> &Qdrant {
        &self.client
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    fn kind(&self) -> &'static str {
        "qdrant"
    }

    async fn ensure_collection(&self, name: &str, dims: usize) -> SealStackResult<()> {
        self.ensure_collection_spec(&crate::CollectionSpec {
            name: name.to_owned(),
            dims,
            distance: Distance::default(),
        })
        .await
    }

    async fn ensure_collection_spec(&self, spec: &crate::CollectionSpec) -> SealStackResult<()> {
        // Idempotent: check first.
        let existing = self
            .client
            .collection_exists(&spec.name)
            .await
            .map_err(SealStackError::backend)?;
        if existing {
            return Ok(());
        }
        let distance = match spec.distance {
            Distance::Cosine => QDistance::Cosine,
            Distance::Dot => QDistance::Dot,
            Distance::Euclidean => QDistance::Euclid,
        };
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&spec.name)
                    .vectors_config(VectorParamsBuilder::new(spec.dims as u64, distance)),
            )
            .await
            .map_err(SealStackError::backend)?;
        tracing::info!(collection = %spec.name, dims = spec.dims, "created qdrant collection");
        Ok(())
    }

    async fn upsert(&self, collection: &str, chunks: Vec<Chunk>) -> SealStackResult<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let points: Vec<PointStruct> = chunks
            .into_iter()
            .map(|c| {
                let mut payload: HashMap<String, QValue> = HashMap::new();
                // Mirror content into payload so search results carry it back.
                payload.insert("content".to_owned(), qvalue_string(&c.content));
                for (k, v) in c.metadata {
                    payload.insert(k, serde_json_to_qvalue(v));
                }
                PointStruct::new(ulid_to_point_id(c.id), c.embedding, payload)
            })
            .collect();

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection, points).wait(true))
            .await
            .map_err(SealStackError::backend)?;
        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        query_vec: Vec<f32>,
        top_k: usize,
        filter: Option<Value>,
    ) -> SealStackResult<Vec<SearchResult>> {
        let limit = u64::try_from(top_k).unwrap_or(16);
        let mut builder = SearchPointsBuilder::new(collection, query_vec, limit).with_payload(true);
        if let Some(f) = filter.and_then(value_to_filter) {
            builder = builder.filter(f);
        }
        let resp = self
            .client
            .search_points(builder)
            .await
            .map_err(SealStackError::backend)?;

        let hits = resp
            .result
            .into_iter()
            .map(|scored| {
                // `Ulid::default()` calls `Ulid::new()` (a fresh clock+RNG ULID)
                // and is only evaluated on the None path; using
                // `unwrap_or_default` here is equivalent to the explicit closure.
                let id = point_id_to_ulid(scored.id.as_ref()).unwrap_or_default();
                let mut metadata = serde_json::Map::new();
                let mut content = String::new();
                for (k, v) in scored.payload {
                    let json = qvalue_to_serde_json(v);
                    if k == "content"
                        && let Value::String(s) = &json
                    {
                        content = s.clone();
                    }
                    metadata.insert(k, json);
                }
                SearchResult {
                    id,
                    score: scored.score,
                    content,
                    metadata,
                }
            })
            .collect();
        Ok(hits)
    }

    async fn delete(&self, collection: &str, ids: Vec<ulid::Ulid>) -> SealStackResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let point_ids: Vec<PointId> = ids.into_iter().map(ulid_to_point_id).collect();
        self.client
            .delete_points(
                DeletePointsBuilder::new(collection)
                    .points(PointsIdsList { ids: point_ids })
                    .wait(true),
            )
            .await
            .map_err(SealStackError::backend)?;
        Ok(())
    }

    async fn count(&self, collection: &str) -> SealStackResult<u64> {
        let resp = self
            .client
            .count(qdrant_client::qdrant::CountPointsBuilder::new(collection).exact(true))
            .await
            .map_err(SealStackError::backend)?;
        Ok(resp.result.map(|r| r.count).unwrap_or(0))
    }

    async fn drop_collection(&self, name: &str) -> SealStackResult<()> {
        self.client
            .delete_collection(name)
            .await
            .map_err(SealStackError::backend)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Point-id conversion
// ---------------------------------------------------------------------------

fn ulid_to_point_id(id: ulid::Ulid) -> PointId {
    // Qdrant point ids are either UUIDs or unsigned 64-bit integers. ULIDs are
    // 128-bit Crockford-base32 strings; we encode them as UUID-shaped strings
    // since ULID and UUID are both 128 bits.
    let uuid = uuid_from_ulid(id);
    PointId {
        point_id_options: Some(PointIdOptions::Uuid(uuid)),
    }
}

fn point_id_to_ulid(id: Option<&PointId>) -> Option<ulid::Ulid> {
    let id = id?;
    match &id.point_id_options {
        Some(PointIdOptions::Uuid(s)) => ulid_from_uuid(s),
        Some(PointIdOptions::Num(n)) => {
            // Legacy numeric ids — fabricate a ULID from the u64 for round-trip
            // stability. Loses high bits but we only hit this for ids inserted
            // by non-SealStack clients.
            Some(ulid::Ulid::from_parts(0, u128::from(*n)))
        }
        None => None,
    }
}

fn uuid_from_ulid(id: ulid::Ulid) -> String {
    let bytes: u128 = id.into();
    let b = bytes.to_be_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

fn ulid_from_uuid(s: &str) -> Option<ulid::Ulid> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        return None;
    }
    let n = u128::from_str_radix(&hex, 16).ok()?;
    Some(ulid::Ulid::from(n))
}

// ---------------------------------------------------------------------------
// serde_json::Value <-> qdrant Value
// ---------------------------------------------------------------------------

fn qvalue_string(s: &str) -> QValue {
    QValue {
        kind: Some(QKind::StringValue(s.to_owned())),
    }
}

fn serde_json_to_qvalue(v: Value) -> QValue {
    let kind = match v {
        Value::Null => QKind::NullValue(0),
        Value::Bool(b) => QKind::BoolValue(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                QKind::IntegerValue(i)
            } else if let Some(f) = n.as_f64() {
                QKind::DoubleValue(f)
            } else {
                QKind::StringValue(n.to_string())
            }
        }
        Value::String(s) => QKind::StringValue(s),
        Value::Array(items) => {
            let list = qdrant_client::qdrant::ListValue {
                values: items.into_iter().map(serde_json_to_qvalue).collect(),
            };
            QKind::ListValue(list)
        }
        Value::Object(map) => {
            let mut fields = HashMap::new();
            for (k, v) in map {
                fields.insert(k, serde_json_to_qvalue(v));
            }
            QKind::StructValue(qdrant_client::qdrant::Struct { fields })
        }
    };
    QValue { kind: Some(kind) }
}

fn qvalue_to_serde_json(v: QValue) -> Value {
    match v.kind {
        None | Some(QKind::NullValue(_)) => Value::Null,
        Some(QKind::BoolValue(b)) => Value::Bool(b),
        Some(QKind::IntegerValue(i)) => Value::from(i),
        Some(QKind::DoubleValue(f)) => {
            serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
        }
        Some(QKind::StringValue(s)) => Value::String(s),
        Some(QKind::ListValue(list)) => {
            Value::Array(list.values.into_iter().map(qvalue_to_serde_json).collect())
        }
        Some(QKind::StructValue(s)) => {
            let mut map = serde_json::Map::new();
            for (k, v) in s.fields {
                map.insert(k, qvalue_to_serde_json(v));
            }
            Value::Object(map)
        }
    }
}

// ---------------------------------------------------------------------------
// JSON filter → qdrant Filter
// ---------------------------------------------------------------------------

/// Translate the narrow JSON filter DSL into a Qdrant `Filter`.
///
/// Returns `None` if the filter is empty or unsupported, in which case the
/// caller passes no filter and may post-filter as a fallback.
fn value_to_filter(v: Value) -> Option<Filter> {
    let obj = v.as_object()?.clone();
    if obj.is_empty() {
        return None;
    }
    let mut must: Vec<Condition> = Vec::new();
    for (field, val) in obj {
        match val {
            Value::String(s) => must.push(Condition::matches(field, s)),
            Value::Bool(b) => must.push(Condition::matches(field, b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    must.push(Condition::matches(field, i));
                }
            }
            Value::Array(items) => {
                // "any of" — collect strings; ignore mixed-type arrays in v0.1.
                let strings: Vec<String> = items
                    .into_iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect();
                if !strings.is_empty() {
                    must.push(Condition::matches(field, strings));
                }
            }
            _ => { /* skip objects/null for v0.1 */ }
        }
    }
    if must.is_empty() {
        None
    } else {
        Some(Filter::must(must))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_uuid_roundtrip() {
        let a = ulid::Ulid::new();
        let uuid = uuid_from_ulid(a);
        let b = ulid_from_uuid(&uuid).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn qvalue_conversion_roundtrip_scalar() {
        let v = Value::String("hello".into());
        let q = serde_json_to_qvalue(v.clone());
        let back = qvalue_to_serde_json(q);
        assert_eq!(v, back);
    }

    #[test]
    fn qvalue_conversion_roundtrip_array() {
        let v = serde_json::json!(["a", "b", "c"]);
        let q = serde_json_to_qvalue(v.clone());
        let back = qvalue_to_serde_json(q);
        assert_eq!(v, back);
    }

    #[test]
    fn empty_filter_yields_none() {
        assert!(value_to_filter(serde_json::json!({})).is_none());
    }

    #[test]
    fn simple_filter_builds_must_clause() {
        let f = value_to_filter(serde_json::json!({ "status": "open" })).unwrap();
        assert_eq!(f.must.len(), 1);
    }
}
