/**
 * Request body for `POST /v1/connectors`.
 */
export interface RegisterConnectorRequest {
  /**
   * Connector kind (`"local-files"`, `"github"`, `"slack"`, `"google-drive"`).
   */
  kind: string;
  /**
   * Qualified schema name this connector binds to.
   */
  schema: string;
  /**
   * Free-shaped connector-specific config (root path, OAuth token, etc.).
   */
  config: {
    [k: string]: unknown;
  };
  [k: string]: unknown;
}
