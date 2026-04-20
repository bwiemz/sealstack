-- Extend cfg_connectors with the columns needed to reconstruct a
-- ConnectorBinding on gateway restart (kind, target schema, interval).
--
-- The original init migration only stored `name` + `config`, which is not
-- enough to instantiate a binding via the connector factory. These four
-- columns close the gap.

ALTER TABLE cfg_connectors
    ADD COLUMN IF NOT EXISTS kind             text  NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS target_namespace text  NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS target_schema    text  NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS tenant           text  NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS interval_secs    bigint;

-- Index to let the hydrate path filter out historical rows where `kind` is
-- still empty (from before this migration ran against a populated DB).
CREATE INDEX IF NOT EXISTS cfg_connectors_kind_idx
    ON cfg_connectors (kind)
    WHERE kind <> '';
