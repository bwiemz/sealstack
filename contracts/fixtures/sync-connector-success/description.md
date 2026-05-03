# sync-connector-success

`POST /v1/connectors/{id}/sync` triggers a synchronous sync run for
the named binding. Response carries the `job_id` so the caller can
correlate logs / receipts emitted during the run. The encoded `id`
in the path is `local-files/examples.Doc`; the slash and dot are
URL-safe in path segments per RFC 3986 §3.3.
