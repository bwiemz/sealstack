# apply-ddl-validation-error

`POST /v1/schemas/examples.Doc/ddl` with malformed DDL. The gateway
returns HTTP 400 with `invalid_argument`; SDK maps this to
`InvalidArgumentError`. Tests the shape of the error envelope on a
4xx that originates from request validation rather than authz.
