/**
 * One contributing source row in a [`ReceiptWire`].
 */
export interface ReceiptSource {
  /**
   * Chunk ID this source resolves to.
   */
  chunk_id: string;
  /**
   * Source URI for the human reader.
   */
  source_uri: string;
  /**
   * Hybrid score for this contribution.
   */
  score: number;
  [k: string]: unknown;
}
