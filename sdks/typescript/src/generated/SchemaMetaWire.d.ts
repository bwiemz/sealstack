/**
 * Response data for `GET /v1/schemas` and `GET /v1/schemas/{q}`.
 *
 * Public projection of `sealstack_engine::SchemaMeta`. Intentionally narrower than the engine's struct: drops `fields`, `relations`, `facets`, `chunked_fields`, and `context` (the SDK surface exposes schema identity + storage targets, not the full schema body). Also flattens `Option<f32> hybrid_alpha` → `f32`; the gateway substitutes the global default when the engine's value is `None`. Keeping `sealstack-api-types` free of engine deps requires the duplication; the gateway maps fields at the response boundary.
 */
export interface SchemaMetaWire {
  /**
   * Namespace, e.g. `"examples"`.
   */
  namespace: string;
  /**
   * Schema name, e.g. `"Doc"`.
   */
  name: string;
  /**
   * Schema-version integer.
   */
  version: number;
  /**
   * Field name used as primary key.
   */
  primary_key: string;
  /**
   * Postgres table name.
   */
  table: string;
  /**
   * Vector store collection name.
   */
  collection: string;
  /**
   * Hybrid score blend factor.
   */
  hybrid_alpha: number;
  [k: string]: unknown;
}
