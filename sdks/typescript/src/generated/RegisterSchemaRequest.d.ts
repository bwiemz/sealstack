/**
 * Request body for `POST /v1/schemas`.
 */
export interface RegisterSchemaRequest {
  /**
   * Schema metadata as emitted by `sealstack_csl::codegen`. Free-shaped here to avoid coupling api-types to the CSL crate; gateway parses into the typed `sealstack_engine::SchemaMeta` internally.
   */
  meta: {
    [k: string]: unknown;
  };
  [k: string]: unknown;
}
