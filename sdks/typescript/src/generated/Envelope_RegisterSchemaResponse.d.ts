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
export interface EnvelopeFor_RegisterSchemaResponse {
  /**
   * Success payload, or `null` on failure.
   */
  data?: RegisterSchemaResponse | null;
  /**
   * Error payload, or `null` on success.
   */
  error?: ErrorDetail | null;
  [k: string]: unknown;
}
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
