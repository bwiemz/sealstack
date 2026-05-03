/**
 * Response data for `POST /v1/query`.
 *
 * Field order and names mirror `sealstack_engine::api::SearchResponse` exactly so the gateway can pass the engine's response through unchanged. SDK contract spec §7 documents this as the canonical wire shape.
 */
export interface QueryResponse {
  /**
   * Receipt ID; resolves via `GET /v1/receipts/{id}`.
   */
  receipt_id: string;
  /**
   * Ranked hits.
   */
  results: QueryHit[];
  [k: string]: unknown;
}
/**
 * One ranked hit in a [`QueryResponse`].
 *
 * Field shape mirrors `sealstack_engine::api::SearchHit` exactly so the gateway can pass engine hits through without per-field conversion.
 */
export interface QueryHit {
  /**
   * Primary key of the matched record.
   */
  id: string;
  /**
   * Combined hybrid score.
   */
  score: number;
  /**
   * Snippet of text likely to have matched.
   */
  excerpt: string;
  /**
   * The full record as a JSON object.
   */
  record: {
    [k: string]: unknown;
  };
  [k: string]: unknown;
}
