/**
 * Response data for `POST /v1/connectors`.
 */
export interface RegisterConnectorResponse {
  /**
   * Connector binding ID (`<kind>/<qualified>`).
   */
  id: string;
  [k: string]: unknown;
}
