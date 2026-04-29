//! In-memory registry of compiled CSL schemas.
//!
//! Populated at boot by walking the compiler's output directory (`out/mcp/*.json`
//! plus a matching `out/schemas/*.json` emitted by `sealstack_csl::codegen`). The engine
//! reads schema metadata every time it dispatches a request; the registry gives
//! it an O(1) lookup keyed on `(namespace, schema)`.
//!
//! The registry is immutable after `Engine::new` returns; hot-reload support
//! (reacting to file-system events in `compile_dir`) is deferred to v0.2.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::api::EngineError;

/// Metadata the engine needs to dispatch requests against one compiled schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaMeta {
    /// CSL namespace (e.g. `acme.crm`).
    pub namespace: String,
    /// Schema name (e.g. `Customer`).
    pub name: String,
    /// Integer version from CSL.
    pub version: u32,
    /// Primary key field name.
    pub primary_key: String,
    /// Ordered field definitions.
    pub fields: Vec<FieldMeta>,
    /// Defined relations. Keyed by relation name.
    pub relations: BTreeMap<String, RelationMeta>,
    /// `@facet` field names in declaration order.
    pub facets: Vec<String>,
    /// The `@chunked` text fields that participate in vector search.
    pub chunked_fields: Vec<String>,
    /// Context block settings that retrieval needs.
    pub context: ContextMeta,
    /// Vector-store collection name for this schema (e.g. `customer_v2`).
    pub collection: String,
    /// Postgres table name (e.g. `customer`).
    pub table: String,
    /// Per-schema hybrid alpha override, if set.
    pub hybrid_alpha: Option<f32>,
}

/// Metadata for one field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMeta {
    /// Field identifier.
    pub name: String,
    /// Postgres column name (usually snake_case of `name`).
    pub column: String,
    /// CSL type, rendered canonically (e.g. `"String"`, `"Ref<User>"`, `"Vector<1024>"`).
    pub ty: String,
    /// Whether the field is `@primary`.
    pub primary: bool,
    /// Whether the field is `@indexed`.
    pub indexed: bool,
    /// Whether the field is `@searchable` (BM25 candidate).
    pub searchable: bool,
    /// Whether the field is `@chunked` (vector candidate).
    pub chunked: bool,
    /// Whether the field is `@facet`.
    pub facet: bool,
    /// Whether the field is optional (`T?`).
    pub optional: bool,
    /// Retrieval boost multiplier if `@boost(factor)` was set.
    pub boost: Option<f32>,
    /// PII tag if `@pii(kind)` was set.
    pub pii: Option<String>,
}

/// Metadata for one relation declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationMeta {
    /// Relation name (the local field name in CSL).
    pub name: String,
    /// Cardinality: `one` or `many`.
    pub kind: RelationKind,
    /// The target schema's namespace.
    pub target_namespace: String,
    /// The target schema's name.
    pub target_schema: String,
    /// Foreign-key column on the target side (from `via`).
    pub foreign_key: String,
}

/// Relation cardinality.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RelationKind {
    /// Exactly one related record.
    One,
    /// Zero or more related records.
    Many,
}

/// Retrieval-relevant fields from the schema's `context { ... }` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMeta {
    /// Embedder identifier (matches an `Embedder::name()`).
    pub embedder: String,
    /// Embedding dimensionality.
    pub vector_dims: usize,
    /// Chunking strategy.
    pub chunking: ChunkingStrategy,
    /// Freshness-decay strategy.
    #[serde(default)]
    pub freshness_decay: FreshnessDecay,
    /// Default top-k for this schema.
    pub default_top_k: Option<usize>,
}

/// Chunking strategy from CSL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChunkingStrategy {
    /// Split at approximate semantic boundaries.
    Semantic {
        /// Max tokens per chunk.
        max_tokens: usize,
        /// Overlap between adjacent chunks.
        #[serde(default)]
        overlap: usize,
    },
    /// Fixed-size character splits.
    Fixed {
        /// Size in characters.
        size: usize,
    },
    /// Recursive splitter with a prioritized separator list.
    Recursive {
        /// Separators in priority order, longest first.
        split: Vec<String>,
        /// Max tokens per chunk.
        max_tokens: usize,
    },
}

impl Default for ChunkingStrategy {
    fn default() -> Self {
        Self::Semantic {
            max_tokens: 512,
            overlap: 64,
        }
    }
}

/// Freshness-decay strategy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FreshnessDecay {
    /// No decay.
    #[default]
    None,
    /// Exponential decay with configurable half-life in seconds.
    Exponential {
        /// Half-life in seconds.
        half_life_secs: u64,
    },
    /// Linear decay across a window in seconds.
    Linear {
        /// Decay window in seconds.
        window_secs: u64,
    },
    /// Step function: (age_cutoff_secs, factor) pairs applied in ascending order.
    Step {
        /// Cliffs in seconds; scores step down at each cutoff.
        cliffs: Vec<u64>,
        /// Corresponding multiplicative factors; must be same length as `cliffs`.
        factors: Vec<f32>,
    },
}

// ---------------------------------------------------------------------------
// Registry.
// ---------------------------------------------------------------------------

/// Thread-safe in-memory registry.
#[derive(Clone, Default)]
pub struct SchemaRegistry {
    inner: Arc<DashMap<(String, String), Arc<SchemaMeta>>>,
}

impl SchemaRegistry {
    /// Empty registry.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Walk a compiler output directory and load every `*.schema.json`.
    ///
    /// The expected layout is:
    ///
    /// ```text
    /// out/
    /// └── schemas/
    ///     ├── acme.crm.customer.schema.json
    ///     └── acme.crm.ticket.schema.json
    /// ```
    pub fn load_from_dir(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let dir = dir.as_ref().join("schemas");
        let registry = Self::empty();
        if !dir.exists() {
            tracing::warn!(path = %dir.display(), "schema directory does not exist; registry is empty");
            return Ok(registry);
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            registry.load_file(&path)?;
        }
        Ok(registry)
    }

    /// Load a single JSON schema file and insert it.
    pub fn load_file(&self, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path)?;
        let meta: SchemaMeta = serde_json::from_slice(&bytes)?;
        self.insert(meta);
        Ok(())
    }

    /// Register a schema in-process (used by tests and by hot-reload).
    pub fn insert(&self, meta: SchemaMeta) {
        let key = (meta.namespace.clone(), meta.name.clone());
        tracing::info!(
            namespace = %meta.namespace, schema = %meta.name, version = meta.version,
            "registered schema",
        );
        self.inner.insert(key, Arc::new(meta));
    }

    /// Look up a schema by `(namespace, name)`.
    pub fn get(&self, namespace: &str, schema: &str) -> Result<Arc<SchemaMeta>, EngineError> {
        self.inner
            .get(&(namespace.into(), schema.into()))
            .map(|r| r.value().clone())
            .ok_or_else(|| EngineError::UnknownSchema {
                namespace: namespace.into(),
                schema: schema.into(),
            })
    }

    /// Resolve a relation defined on `(namespace, schema)`.
    pub fn resolve_relation(
        &self,
        namespace: &str,
        schema: &str,
        relation: &str,
    ) -> Result<(Arc<SchemaMeta>, RelationMeta), EngineError> {
        let meta = self.get(namespace, schema)?;
        let rel = meta.relations.get(relation).cloned().ok_or_else(|| {
            EngineError::InvalidArgument(format!("unknown relation `{relation}`"))
        })?;
        Ok((meta, rel))
    }

    /// Iterate over every registered schema.
    pub fn iter(&self) -> Vec<Arc<SchemaMeta>> {
        self.inner.iter().map(|e| e.value().clone()).collect()
    }

    /// Total schema count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the registry has any entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl SchemaRegistry {
    /// Expose the compile directory this registry was loaded from (diagnostic).
    ///
    /// Not tracked in v0.1; always returns `None`. Reserved for the hot-reload path.
    #[must_use]
    pub fn source_dir(&self) -> Option<PathBuf> {
        None
    }

    /// Pull every schema from `sealstack_schemas` into this in-memory registry.
    ///
    /// Rows that deserialize cleanly are inserted; rows with the same
    /// `(namespace, name)` as an existing entry are overwritten (newer on disk
    /// beats older on disk because `list_schemas` returns only the highest
    /// version per name).
    pub async fn hydrate_from_store(&self, store: &crate::store::Store) -> anyhow::Result<usize> {
        let metas = store.list_schemas().await?;
        let n = metas.len();
        for meta in metas {
            self.insert(meta);
        }
        tracing::info!(count = n, "hydrated schemas from store");
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SchemaMeta {
        SchemaMeta {
            namespace: "acme.crm".into(),
            name: "Customer".into(),
            version: 1,
            primary_key: "id".into(),
            fields: vec![FieldMeta {
                name: "id".into(),
                column: "id".into(),
                ty: "Ulid".into(),
                primary: true,
                indexed: false,
                searchable: false,
                chunked: false,
                facet: false,
                optional: false,
                boost: None,
                pii: None,
            }],
            relations: BTreeMap::new(),
            facets: vec![],
            chunked_fields: vec![],
            context: ContextMeta {
                embedder: "stub".into(),
                vector_dims: 64,
                chunking: ChunkingStrategy::default(),
                freshness_decay: FreshnessDecay::None,
                default_top_k: Some(10),
            },
            collection: "customer_v1".into(),
            table: "customer".into(),
            hybrid_alpha: None,
        }
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let r = SchemaRegistry::empty();
        r.insert(sample());
        let got = r.get("acme.crm", "Customer").unwrap();
        assert_eq!(got.primary_key, "id");
    }

    #[test]
    fn unknown_schema_error() {
        let r = SchemaRegistry::empty();
        let err = r.get("nope", "Missing").unwrap_err();
        assert!(matches!(err, EngineError::UnknownSchema { .. }));
    }
}
