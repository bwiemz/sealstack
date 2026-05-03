/**
 * Response data for `POST /v1/schemas/{qualified}/ddl`.
 */
export interface ApplyDdlResponse {
  /**
   * Number of statements applied.
   */
  applied: number;
  [k: string]: unknown;
}
