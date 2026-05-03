/**
 * Public projection of `sealstack_ingest::ConnectorBindingInfo` for the REST surface. Intentionally diverges in two places: renames `connector` → `kind` for SDK ergonomics, and adds an `enabled: bool` that the gateway computes from the binding's runtime state (no engine field to mirror directly). The gateway maps fields at the response boundary.
 */
export interface ConnectorBindingWire {
  /**
   * Binding ID.
   */
  id: string;
  /**
   * Connector kind.
   */
  kind: string;
  /**
   * Qualified schema name.
   */
  schema: string;
  /**
   * Whether the binding is enabled.
   */
  enabled: boolean;
  [k: string]: unknown;
}
