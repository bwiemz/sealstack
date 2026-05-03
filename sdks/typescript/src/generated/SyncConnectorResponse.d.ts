/**
 * Response data for `POST /v1/connectors/{id}/sync`.
 */
export interface SyncConnectorResponse {
  /**
   * Job identifier for the sync run.
   */
  job_id: string;
  [k: string]: unknown;
}
