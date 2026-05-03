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
