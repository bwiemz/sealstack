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
