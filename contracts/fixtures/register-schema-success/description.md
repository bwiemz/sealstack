# register-schema-success

`POST /v1/schemas` registers a compiled schema with the engine. Body
mirrors the shape `sealstack_csl::codegen` emits; gateway parses it
into the typed `SchemaMeta` internally. Response carries the qualified
name (`<namespace>.<name>`) the SDK should reference for subsequent
DDL and connector binding calls.
