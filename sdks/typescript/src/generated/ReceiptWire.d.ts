/**
 * Public projection of `sealstack_engine::Receipt` for the REST surface. Intentionally narrower than the engine's internal Receipt: drops `qualified_schema`, `tool`, `arguments`, `policies_applied`, and `timings_ms`; renames `created_at` → `issued_at`; and surfaces `tenant` at the top level (engine has it nested in `caller`). The gateway maps fields at the response boundary.
 */
export interface ReceiptWire {
  /**
   * Receipt ID (ULID).
   */
  id: string;
  /**
   * Caller identity at query time.
   */
  caller_id: string;
  /**
   * Tenant the query ran against.
   */
  tenant: string;
  /**
   * Source records that contributed to the answer.
   */
  sources: ReceiptSource[];
  /**
   * Issue timestamp (RFC 3339).
   */
  issued_at: string;
  [k: string]: unknown;
}
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
