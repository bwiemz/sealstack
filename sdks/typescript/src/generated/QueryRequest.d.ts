/**
 * Request body for `POST /v1/query`.
 */
export interface QueryRequest {
  /**
   * Qualified schema name, e.g. `"examples.Doc"`.
   */
  schema: string;
  /**
   * Query string (natural-language or keywords).
   */
  query: string;
  /**
   * Cap on results; `None` defaults server-side.
   */
  top_k?: number | null;
  /**
   * Filter expression; structure depends on schema's facet declarations.
   */
  filters?: {
    [k: string]: unknown;
  };
  [k: string]: unknown;
}
