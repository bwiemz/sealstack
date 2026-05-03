/**
 * Request body for `POST /v1/schemas/{qualified}/ddl`.
 */
export interface ApplyDdlRequest {
  /**
   * Postgres DDL text (CREATE TABLE / CREATE INDEX / ...).
   */
  ddl: string;
  [k: string]: unknown;
}
