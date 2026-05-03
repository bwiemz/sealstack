/**
 * Closed-set error code emitted by the gateway. SDKs map each variant to a typed exception class (see SDK contract spec §8).
 */
export type ErrorCode =
  | "not_found"
  | "unknown_schema"
  | "unauthorized"
  | "policy_denied"
  | "invalid_argument"
  | "rate_limited"
  | "backend";

/**
 * Discriminated-union response envelope. On success, `data` is `T` and `error` is `null`. On failure, `data` is `null` and `error` carries a code from [`ErrorCode`] and a human-readable message.
 */
export interface EnvelopeFor_QueryResponse {
  /**
   * Success payload, or `null` on failure.
   */
  data?: QueryResponse | null;
  /**
   * Error payload, or `null` on success.
   */
  error?: ErrorDetail | null;
  [k: string]: unknown;
}
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
/**
 * Error detail returned in [`Envelope::error`].
 *
 * v0.3 wire shape is `{ code, message }` only. SDK contract spec §8 lists per-class typed attributes (`PolicyDeniedError.predicate`, `InvalidArgumentError.field`/`reason`, etc.) — those are populated by the SDK from response headers (`X-Request-Id` → `BackendError.request_id`; `Retry-After` → `RateLimitedError.retry_after`) or are deferred to a v0.4 envelope addition that adds an `Option<Value> details` field. The `message` field is human-readable and SDKs must not parse it.
 */
export interface ErrorDetail {
  /**
   * Closed-set error code; see [`ErrorCode`].
   */
  code: ErrorCode;
  /**
   * Human-readable message. Not part of the contract; do not parse.
   */
  message: string;
  [k: string]: unknown;
}
