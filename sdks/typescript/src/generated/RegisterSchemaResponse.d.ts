/**
 * Response data for `POST /v1/schemas`.
 */
export interface RegisterSchemaResponse {
  /**
   * Qualified schema name (`<namespace>.<name>`).
   */
  qualified: string;
  [k: string]: unknown;
}
