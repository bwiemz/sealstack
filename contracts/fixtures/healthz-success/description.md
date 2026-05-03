# healthz-success

`GET /healthz` (note: not `/v1/`-prefixed) returns the gateway's
liveness status. Status discriminator is the `HealthStatusKind` enum;
`ok` indicates ready to serve traffic.
