# Connectors

Each connector is an independent crate implementing the
`sealstack_connector_sdk::Connector` trait.

## Writing a new connector

1. `cargo new --lib connectors/<your-connector>`
2. Add it to the workspace `members` in the root `Cargo.toml`.
3. Depend on `sealstack-connector-sdk` and implement `Connector`.
4. Register it in the ingest runtime via the plugin loader.

See `connectors/local-files` for a minimal reference implementation.
