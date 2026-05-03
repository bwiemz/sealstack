# query-policy-denied

`POST /v1/query` against a schema whose policy bundle denies the caller.
The gateway returns HTTP 403 with the `policy_denied` error code; the
SDK maps this to `PolicyDeniedError`. Tests the error-envelope side of
the discriminated-union response shape.
