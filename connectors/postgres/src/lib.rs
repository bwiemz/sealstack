//! Postgres scrape connector.
//!
//! Projects rows from a configured Postgres table into [`Resource`]s by
//! running a single `SELECT` over a curated column list. Each row becomes
//! one resource; designated text columns are concatenated into the body
//! field, the primary-key column becomes the source id, and remaining
//! columns are surfaced as JSON metadata.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "dsn":             "postgres://user:pass@host:5432/db",
//!   "table":           "support_tickets",
//!   "id_column":       "id",
//!   "body_columns":    ["title", "body"],
//!   "title_column":    "title",
//!   "updated_at_column": "updated_at",
//!   "limit":           10000,
//!   "max_body_bytes":  1048576
//! }
//! ```
//!
//! Identifiers (`table`, `id_column`, every entry in `body_columns`, and
//! the optional `title_column`/`updated_at_column`) are validated against
//! `^[a-zA-Z_][a-zA-Z0-9_]*$` and rejected otherwise — Postgres won't bind
//! identifier positions, so a hand-rolled allowlist is the only defense.
//!
//! TLS: the DSN is passed verbatim to `sqlx` and is expected to specify
//! `sslmode=require` or stronger for any non-localhost host. The connector
//! does *not* implicitly add `sslmode` flags; if you want enforcement, set
//! it in the DSN.
//!
//! # Limitations (v0.3)
//!
//! - Single-table scrape. No joins, no SQL injection escape hatch.
//! - No `LISTEN/NOTIFY` push path; `subscribe()` returns `Ok(None)`.
//! - `fetch()` re-runs the projection with `WHERE {id_column} = $1`.

use async_trait::async_trait;
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
use serde::Deserialize;
use serde_json::Value;
use sqlx::Column;
use sqlx::Row;
use sqlx::TypeInfo;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::OnceLock;
use time::OffsetDateTime;

/// Default cap on per-row body size, in bytes (matches `local-files` default).
const DEFAULT_MAX_BODY_BYTES: usize = 1_048_576;
/// Default cap on rows returned per sync. Keeps unbounded queries bounded.
const DEFAULT_LIMIT: u64 = 10_000;
/// Concurrent-connection cap.
const POOL_MAX_CONNECTIONS: u32 = 8;

/// Connector configuration.
///
/// Mirrors the JSON shape documented on the module — `serde(deny_unknown_fields)`
/// is intentional so typos in connector configs surface at registration time
/// instead of silently producing empty syncs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Postgres connection string. Treat as a secret.
    pub dsn: String,
    /// Source table the projection reads from.
    pub table: String,
    /// Column holding the row primary key. Bound as `text` in `fetch()`.
    pub id_column: String,
    /// Text columns concatenated (with `\n\n`) into [`Resource::body`].
    pub body_columns: Vec<String>,
    /// Optional column used for the resource display title.
    #[serde(default)]
    pub title_column: Option<String>,
    /// Optional column used as the source-side `updated_at` timestamp.
    /// When unset every emitted row gets the current UTC time.
    #[serde(default)]
    pub updated_at_column: Option<String>,
    /// Cap on rows returned per sync. Defaults to [`DEFAULT_LIMIT`].
    #[serde(default)]
    pub limit: Option<u64>,
    /// Cap on per-row body size in bytes. Defaults to [`DEFAULT_MAX_BODY_BYTES`].
    #[serde(default)]
    pub max_body_bytes: Option<usize>,
}

/// Postgres scrape connector. Cheap to clone via the inner `PgPool`.
#[derive(Clone)]
pub struct PostgresConnector {
    config: Config,
    pool: PgPool,
    /// Cached select-projection clause: `id, body_col1, body_col2, ...`.
    /// Built once at construction so the hot path stays string-free.
    projection: String,
}

impl PostgresConnector {
    /// Build from a JSON config payload (the shape used by `sealstack
    /// connector add`).
    ///
    /// # Errors
    ///
    /// Returns [`SealStackError::Config`] on a missing field, an unknown
    /// field, or an identifier that fails the allowlist. Returns
    /// [`SealStackError::Backend`] if the DSN is malformed (actual TCP
    /// connections are lazy — see [`Self::new`]).
    pub fn from_json(v: &Value) -> SealStackResult<Self> {
        let config: Config = serde_json::from_value(v.clone())
            .map_err(|e| SealStackError::Config(format!("postgres connector config: {e}")))?;
        Self::new(config)
    }

    /// Build from a typed [`Config`]. Validates identifiers and constructs
    /// a lazy connection pool; the first TCP connect happens on the first
    /// `list()` / `fetch()` / `healthcheck()` call so connector registration
    /// stays cheap and synchronous. Prefer [`Self::from_json`] from
    /// production paths.
    ///
    /// # Errors
    ///
    /// See [`Self::from_json`].
    pub fn new(config: Config) -> SealStackResult<Self> {
        validate_ident("table", &config.table)?;
        validate_ident("id_column", &config.id_column)?;
        if config.body_columns.is_empty() {
            return Err(SealStackError::Config(
                "postgres connector requires at least one `body_columns` entry".into(),
            ));
        }
        for col in &config.body_columns {
            validate_ident("body_columns", col)?;
        }
        if let Some(t) = &config.title_column {
            validate_ident("title_column", t)?;
        }
        if let Some(u) = &config.updated_at_column {
            validate_ident("updated_at_column", u)?;
        }

        let mut projection_cols: Vec<&str> = Vec::new();
        projection_cols.push(config.id_column.as_str());
        for col in &config.body_columns {
            projection_cols.push(col.as_str());
        }
        if let Some(t) = &config.title_column {
            projection_cols.push(t.as_str());
        }
        if let Some(u) = &config.updated_at_column {
            projection_cols.push(u.as_str());
        }
        // Deduplicate while preserving order.
        let mut seen = std::collections::HashSet::new();
        let projection = projection_cols
            .into_iter()
            .filter(|c| seen.insert(c.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        let pool = PgPoolOptions::new()
            .max_connections(POOL_MAX_CONNECTIONS)
            .connect_lazy(&config.dsn)
            .map_err(|e| SealStackError::Backend(format!("postgres pool: {e}")))?;

        Ok(Self {
            config,
            pool,
            projection,
        })
    }

    /// Row → Resource projection. Shared between `list` and `fetch` so the
    /// two surfaces always agree on the shape.
    fn row_to_resource(&self, row: &sqlx::postgres::PgRow) -> SealStackResult<Resource> {
        let id = stringify_column(row, &self.config.id_column).ok_or_else(|| {
            SealStackError::Backend(format!(
                "row in `{}` has NULL `{}` (primary key)",
                self.config.table, self.config.id_column,
            ))
        })?;

        let max_body = self.config.max_body_bytes.unwrap_or(DEFAULT_MAX_BODY_BYTES);

        let mut body_parts: Vec<String> = Vec::with_capacity(self.config.body_columns.len());
        for col in &self.config.body_columns {
            if let Some(v) = stringify_column(row, col) {
                body_parts.push(v);
            }
        }
        let mut body = body_parts.join("\n\n");
        if body.len() > max_body {
            tracing::debug!(
                row_id = %id,
                table = %self.config.table,
                bytes = body.len(),
                cap = max_body,
                "row body exceeds cap; truncating",
            );
            body.truncate(max_body);
        }

        let title = self
            .config
            .title_column
            .as_ref()
            .and_then(|t| stringify_column(row, t));

        let source_updated_at = self
            .config
            .updated_at_column
            .as_ref()
            .and_then(|c| row.try_get::<OffsetDateTime, _>(c.as_str()).ok())
            .unwrap_or_else(OffsetDateTime::now_utc);

        let metadata = serde_json::Map::from_iter([
            ("table".into(), Value::String(self.config.table.clone())),
            (
                "source_pk".into(),
                Value::String(format!("{}={id}", self.config.id_column)),
            ),
        ]);

        Ok(Resource {
            id: ResourceId::new(format!("postgres://{}/{}", self.config.table, id)),
            kind: format!("row:{}", self.config.table),
            title,
            body,
            metadata,
            // DB-side ACLs don't project cleanly to source-side principals
            // without a mapping table the operator hasn't given us. Treat as
            // anyone-read at the connector boundary; the engine's CSL policy
            // is the actual access-control surface.
            permissions: vec![PermissionPredicate::public_read()],
            source_updated_at,
        })
    }
}

#[async_trait]
impl Connector for PostgresConnector {
    fn name(&self) -> &str {
        "postgres"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        // SAFETY against identifier injection: every column and table name
        // was validated in `new()` via `validate_ident`. Bound parameters
        // would be safer but Postgres doesn't accept binds for identifier
        // positions.
        let limit = self.config.limit.unwrap_or(DEFAULT_LIMIT);
        let sql = format!(
            "SELECT {projection} FROM {table} LIMIT {limit}",
            projection = self.projection,
            table = self.config.table,
        );

        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SealStackError::Backend(format!("postgres list: {e}")))?;

        let mut out: Vec<Resource> = Vec::with_capacity(rows.len());
        for row in &rows {
            match self.row_to_resource(row) {
                Ok(r) => out.push(r),
                Err(e) => {
                    tracing::warn!(error = %e, "skipping malformed row");
                }
            }
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        // Resource ids are `postgres://{table}/{pk_value}`. Extract the pk.
        let prefix = format!("postgres://{}/", self.config.table);
        let pk = id.as_str().strip_prefix(&prefix).ok_or_else(|| {
            SealStackError::NotFound(format!(
                "id `{id}` doesn't match this connector's table `{}`",
                self.config.table,
            ))
        })?;

        let sql = format!(
            "SELECT {projection} FROM {table} WHERE {id_col}::text = $1 LIMIT 1",
            projection = self.projection,
            table = self.config.table,
            id_col = self.config.id_column,
        );
        let row = sqlx::query(&sql)
            .bind(pk)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| SealStackError::Backend(format!("postgres fetch: {e}")))?
            .ok_or_else(|| SealStackError::NotFound(format!("no row with pk {pk}")))?;

        self.row_to_resource(&row)
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| SealStackError::Backend(format!("postgres healthcheck: {e}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Postgres identifier allowlist regex, compiled once.
fn ident_regex() -> &'static IdentMatcher {
    static R: OnceLock<IdentMatcher> = OnceLock::new();
    R.get_or_init(IdentMatcher::new)
}

/// Tiny hand-rolled regex stand-in. Avoids the `regex` crate dep for a
/// pattern that's a fixed character class.
struct IdentMatcher;

impl IdentMatcher {
    fn new() -> Self {
        Self
    }

    fn is_valid(&self, s: &str) -> bool {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
}

fn validate_ident(field: &str, value: &str) -> SealStackResult<()> {
    if value.len() > 63 {
        return Err(SealStackError::Config(format!(
            "{field}: identifier `{value}` exceeds Postgres's 63-byte cap",
        )));
    }
    if !ident_regex().is_valid(value) {
        return Err(SealStackError::Config(format!(
            "{field}: identifier `{value}` doesn't match `^[a-zA-Z_][a-zA-Z0-9_]*$`",
        )));
    }
    Ok(())
}

/// Best-effort projection of an arbitrary column value to a Rust string.
///
/// Tries known scalar types in turn — text, bigint, integer, double — and
/// falls back to a debug rendering of the raw `PgValueRef`. Returns `None`
/// when the column is NULL.
fn stringify_column(row: &sqlx::postgres::PgRow, name: &str) -> Option<String> {
    if let Ok(v) = row.try_get::<Option<String>, _>(name) {
        return v;
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(name) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(name) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(name) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(name) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<serde_json::Value>, _>(name) {
        return v.map(|x| x.to_string());
    }
    // Last-resort: lookup the column's declared Postgres type and report it
    // in the warning trace so operators know what they need to handle.
    let col = row.columns().iter().find(|c| c.name() == name);
    let ty = col
        .map(|c| c.type_info().name().to_string())
        .unwrap_or_else(|| "<unknown>".into());
    tracing::warn!(column = name, ty = %ty, "unhandled column type; skipping");
    None
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_matcher_accepts_well_formed_identifiers() {
        let m = ident_regex();
        for ok in ["t", "_t", "table_name", "t1", "_1", "T", "snake_case_99"] {
            assert!(m.is_valid(ok), "should accept `{ok}`");
        }
    }

    #[test]
    fn ident_matcher_rejects_injection_attempts() {
        let m = ident_regex();
        for bad in [
            "",
            "1table",
            "table-name",
            "table name",
            "table;drop",
            "\"table\"",
            "table'",
            "table.col",
            "👻",
        ] {
            assert!(!m.is_valid(bad), "should reject `{bad}`");
        }
    }

    #[test]
    fn validate_ident_caps_at_63_bytes() {
        let long = "a".repeat(64);
        let err = validate_ident("table", &long).expect_err("too long");
        assert!(matches!(err, SealStackError::Config(_)));
    }

    #[test]
    fn config_deserialize_minimal() {
        let json = serde_json::json!({
            "dsn":          "postgres://localhost/db",
            "table":        "tickets",
            "id_column":    "id",
            "body_columns": ["body"],
        });
        let c: Config = serde_json::from_value(json).expect("parse");
        assert_eq!(c.table, "tickets");
        assert_eq!(c.body_columns, vec!["body".to_string()]);
        assert!(c.title_column.is_none());
    }

    #[test]
    fn config_rejects_unknown_fields() {
        let json = serde_json::json!({
            "dsn":           "postgres://localhost/db",
            "table":         "tickets",
            "id_column":     "id",
            "body_columns":  ["body"],
            "secret_typo":   "oops",
        });
        let err: Result<Config, _> = serde_json::from_value(json);
        assert!(err.is_err(), "unknown field should be rejected");
    }
}
