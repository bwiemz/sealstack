//! Postgres pool and migration management.
//!
//! Wraps `sqlx` with:
//!
//! * A typed [`Store`] struct that owns the pool.
//! * Boot-time migration execution via the `sqlx::migrate!` macro pointing at
//!   the crate's `migrations/` directory.
//! * A helper for applying CSL-generated DDL at runtime (per-schema migrations
//!   live outside the `migrations/` directory and are applied separately by
//!   [`Store::apply_schema_ddl`]).

use std::time::Duration;

use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};

use crate::api::EngineError;
use crate::schema_registry::SchemaMeta;

/// Persisted-form view of one row in `sealstack_connectors`. The engine stores only
/// the portable fields; wiring them back to an `Arc<dyn Connector>` is the
/// gateway's job (via its connector factory).
#[derive(Debug, Clone)]
pub struct PersistedConnector {
    /// Stable binding id (`"<connector>/<namespace>.<schema>"`).
    pub id: String,
    /// Connector kind (e.g. `"local-files"`).
    pub kind: String,
    /// Target schema namespace.
    pub target_namespace: String,
    /// Target schema name.
    pub target_schema: String,
    /// Tenant to stamp every ingested row with.
    pub tenant: String,
    /// Connector config (opaque JSON).
    pub config: Value,
    /// Optional sync cadence in seconds.
    pub interval_secs: Option<u64>,
}

/// Thin wrapper around the Postgres pool.
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connect to the database and run engine-level migrations.
    ///
    /// Engine migrations live in the `migrations/` directory of this crate and
    /// cover the engine's *own* tables (schemas, connectors, receipts). Per-CSL
    /// schema DDL is a separate concern; see [`Store::apply_schema_ddl`].
    pub async fn connect(database_url: &str, pool_size: u32) -> Result<Self, EngineError> {
        let pool = PgPoolOptions::new()
            .max_connections(pool_size)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await
            .map_err(EngineError::backend)?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(EngineError::backend)?;

        tracing::info!("engine migrations applied");
        Ok(Self { pool })
    }

    /// Start a transaction for multi-statement operations.
    pub async fn begin(&self) -> Result<Transaction<'_, Postgres>, EngineError> {
        self.pool.begin().await.map_err(EngineError::backend)
    }

    /// Access the underlying pool.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ----- Schema persistence ------------------------------------------------

    /// Insert or update one schema. Keyed on `(namespace, name, version)`.
    pub async fn put_schema(&self, meta: &SchemaMeta) -> Result<(), EngineError> {
        let definition =
            serde_json::to_value(meta).map_err(|e| EngineError::backend(e.to_string()))?;
        sqlx::query(
            "INSERT INTO sealstack_schemas (namespace, name, version, definition) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (namespace, name, version) \
             DO UPDATE SET definition = EXCLUDED.definition",
        )
        .bind(&meta.namespace)
        .bind(&meta.name)
        .bind(i32::try_from(meta.version).unwrap_or(i32::MAX))
        .bind(&definition)
        .execute(&self.pool)
        .await
        .map_err(EngineError::backend)?;
        Ok(())
    }

    /// Return the latest version of every registered schema.
    ///
    /// When multiple rows exist for a single `(namespace, name)` we keep the
    /// one with the largest `version`.
    pub async fn list_schemas(&self) -> Result<Vec<SchemaMeta>, EngineError> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (namespace, name) definition \
             FROM sealstack_schemas \
             ORDER BY namespace, name, version DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(EngineError::backend)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let def: Value = row.try_get("definition").map_err(EngineError::backend)?;
            match serde_json::from_value::<SchemaMeta>(def) {
                Ok(m) => out.push(m),
                Err(e) => tracing::warn!(error = %e, "skipping malformed schema row"),
            }
        }
        Ok(out)
    }

    // ----- Connector persistence ---------------------------------------------

    /// Insert or update one connector binding.
    pub async fn put_connector(&self, binding: &PersistedConnector) -> Result<(), EngineError> {
        // `name` has a legacy NOT NULL constraint from the init migration and
        // has no reader today; we seed it with `kind` on insert but DO NOT
        // touch it on update so any future human-readable label wins.
        sqlx::query(
            "INSERT INTO sealstack_connectors \
                 (id, name, kind, target_namespace, target_schema, tenant, config, interval_secs) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (id) DO UPDATE SET \
                 kind              = EXCLUDED.kind, \
                 target_namespace  = EXCLUDED.target_namespace, \
                 target_schema     = EXCLUDED.target_schema, \
                 tenant            = EXCLUDED.tenant, \
                 config            = EXCLUDED.config, \
                 interval_secs     = EXCLUDED.interval_secs",
        )
        .bind(&binding.id)
        .bind(&binding.kind)
        .bind(&binding.kind)
        .bind(&binding.target_namespace)
        .bind(&binding.target_schema)
        .bind(&binding.tenant)
        .bind(&binding.config)
        .bind(
            binding
                .interval_secs
                .map(|s| i64::try_from(s).unwrap_or(i64::MAX)),
        )
        .execute(&self.pool)
        .await
        .map_err(EngineError::backend)?;
        Ok(())
    }

    /// List every enabled connector binding in the database.
    ///
    /// Rows with an empty `kind` (pre-migration leftovers) are skipped; without
    /// a kind the connector factory has nothing to dispatch on.
    pub async fn list_connectors(&self) -> Result<Vec<PersistedConnector>, EngineError> {
        let rows = sqlx::query(
            "SELECT id, kind, target_namespace, target_schema, tenant, config, interval_secs \
             FROM sealstack_connectors \
             WHERE enabled AND kind <> '' \
             ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(EngineError::backend)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(PersistedConnector {
                id: row.try_get("id").map_err(EngineError::backend)?,
                kind: row.try_get("kind").map_err(EngineError::backend)?,
                target_namespace: row
                    .try_get("target_namespace")
                    .map_err(EngineError::backend)?,
                target_schema: row.try_get("target_schema").map_err(EngineError::backend)?,
                tenant: row.try_get("tenant").map_err(EngineError::backend)?,
                config: row.try_get("config").map_err(EngineError::backend)?,
                interval_secs: row
                    .try_get::<Option<i64>, _>("interval_secs")
                    .map_err(EngineError::backend)?
                    .map(|n| u64::try_from(n).unwrap_or(0)),
            });
        }
        Ok(out)
    }

    /// Remove one connector binding. Returns `true` if a row was deleted.
    pub async fn delete_connector(&self, id: &str) -> Result<bool, EngineError> {
        let res = sqlx::query("DELETE FROM sealstack_connectors WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(EngineError::backend)?;
        Ok(res.rows_affected() > 0)
    }

    /// Apply a per-schema DDL bundle produced by `sealstack_csl::codegen::sql`.
    ///
    /// The input is the contents of a `NNNN_up.sql` file. Statements are
    /// executed in one transaction; on failure the whole file is rolled back.
    pub async fn apply_schema_ddl(&self, ddl: &str) -> Result<(), EngineError> {
        let mut tx = self.begin().await?;
        for statement in split_sql(ddl) {
            sqlx::query(&statement)
                .execute(&mut *tx)
                .await
                .map_err(EngineError::backend)?;
        }
        tx.commit().await.map_err(EngineError::backend)?;
        Ok(())
    }
}

/// Extremely simple SQL statement splitter.
///
/// Splits on `;` at statement boundaries while honoring single-quoted strings,
/// dollar-quoted strings (`$tag$...$tag$`), and line comments. This is *not* a
/// full SQL parser; it handles the statements our codegen emits (CREATE TABLE,
/// CREATE INDEX, CREATE FUNCTION with dollar-quoted bodies, ALTER, COMMENT ON).
fn split_sql(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_str = false;
    let mut dollar_tag: Option<String> = None;
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        let c = bytes[i] as char;

        // Line comments
        if !in_str && dollar_tag.is_none() && c == '-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Dollar-quoted strings: $$...$$ or $tag$...$tag$
        if !in_str && c == '$' {
            if let Some(close) = dollar_tag.as_deref() {
                // Inside dollar quote — check for close.
                if input[i..].starts_with(close) {
                    current.push_str(close);
                    i += close.len();
                    dollar_tag = None;
                    continue;
                }
            } else {
                // Possibly opening a new dollar-quote.
                let remaining = &input[i + 1..];
                if let Some(end_tag) = remaining.find('$') {
                    let tag = &remaining[..end_tag];
                    if tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        let full = format!("${tag}$");
                        current.push_str(&full);
                        i += full.len();
                        dollar_tag = Some(full);
                        continue;
                    }
                }
            }
        }

        // Single-quoted strings
        if dollar_tag.is_none() && c == '\'' {
            in_str = !in_str;
            current.push(c);
            i += 1;
            continue;
        }

        // Statement terminator
        if !in_str && dollar_tag.is_none() && c == ';' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_owned());
            }
            current.clear();
            i += 1;
            continue;
        }

        current.push(c);
        i += 1;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_sql;

    #[test]
    fn splits_basic_statements() {
        let sql = "CREATE TABLE a (id int); CREATE TABLE b (id int);";
        let v = split_sql(sql);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn ignores_semicolons_in_strings() {
        let sql = "INSERT INTO t VALUES ('hello;world'); SELECT 1;";
        let v = split_sql(sql);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn handles_dollar_quotes() {
        let sql = r#"
            CREATE FUNCTION f() RETURNS void AS $body$
                BEGIN
                  RAISE NOTICE 'has ; semicolons';
                END;
            $body$ LANGUAGE plpgsql;
            SELECT 1;
        "#;
        let v = split_sql(sql);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn ignores_line_comments() {
        let sql = "SELECT 1; -- trailing comment; fake\nSELECT 2;";
        let v = split_sql(sql);
        assert_eq!(v.len(), 2);
    }
}
