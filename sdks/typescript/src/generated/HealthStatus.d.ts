/**
 * Response data for `GET /healthz` and `GET /readyz`.
 */
export type HealthStatusKind = "ok" | "starting";

/**
 * Body shape for health endpoints.
 */
export interface HealthStatus {
  /**
   * Status discriminator.
   */
  status: HealthStatusKind;
  [k: string]: unknown;
}
