-- Signet engine — initial migration.
--
-- Creates the engine's own control-plane tables. Per-schema tables generated
-- from CSL live in separate migrations applied via Store::apply_schema_ddl.

-- Schema registry mirror (populated by signet-cli when CSL is applied).
CREATE TABLE IF NOT EXISTS signet_schemas (
    namespace   text        NOT NULL,
    name        text        NOT NULL,
    version     integer     NOT NULL,
    definition  jsonb       NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (namespace, name, version)
);

-- Connector instances.
CREATE TABLE IF NOT EXISTS signet_connectors (
    id             text         PRIMARY KEY,
    name           text         NOT NULL,
    config         jsonb        NOT NULL,
    enabled        boolean      NOT NULL DEFAULT true,
    last_sync_at   timestamptz,
    created_at     timestamptz  NOT NULL DEFAULT now()
);

-- Receipts.
CREATE TABLE IF NOT EXISTS signet_receipts (
    id                text         PRIMARY KEY,
    caller_id         text         NOT NULL,
    qualified_schema  text         NOT NULL,
    tool              text         NOT NULL,
    body              jsonb        NOT NULL,
    created_at        timestamptz  NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS signet_receipts_caller_idx       ON signet_receipts (caller_id);
CREATE INDEX IF NOT EXISTS signet_receipts_created_at_idx   ON signet_receipts (created_at);
CREATE INDEX IF NOT EXISTS signet_receipts_qualified_idx    ON signet_receipts (qualified_schema);

-- Ingestion cursor per connector (last offset / ETag / timestamp).
CREATE TABLE IF NOT EXISTS signet_ingest_state (
    connector_id      text         NOT NULL,
    resource_kind     text         NOT NULL,
    cursor            text,
    last_seen_at      timestamptz  NOT NULL DEFAULT now(),
    PRIMARY KEY (connector_id, resource_kind)
);

-- Lineage edges: chunk → source record + metadata. Useful for receipts and
-- for the "why did I see this?" explanation UI.
CREATE TABLE IF NOT EXISTS signet_lineage (
    chunk_id          text         PRIMARY KEY,
    qualified_schema  text         NOT NULL,
    record_id         text         NOT NULL,
    connector_id      text,
    source_path       text,
    created_at        timestamptz  NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS signet_lineage_record_idx ON signet_lineage (qualified_schema, record_id);

-- Sessions (MCP). Stored here rather than Redis when Redis is not configured.
CREATE TABLE IF NOT EXISTS signet_mcp_sessions (
    id           text         PRIMARY KEY,
    caller_id    text         NOT NULL,
    created_at   timestamptz  NOT NULL DEFAULT now(),
    expires_at   timestamptz  NOT NULL
);

CREATE INDEX IF NOT EXISTS signet_mcp_sessions_expires_idx ON signet_mcp_sessions (expires_at);
