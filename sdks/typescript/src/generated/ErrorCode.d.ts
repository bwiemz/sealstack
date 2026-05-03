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
