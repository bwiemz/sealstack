# register-connector-success

`POST /v1/connectors` registers a connector binding against an
already-registered schema. The response carries the binding id —
shape `<kind>/<qualified-schema>` — which the SDK uses for subsequent
sync calls.
