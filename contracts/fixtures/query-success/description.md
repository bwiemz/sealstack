# query-success

`POST /v1/query` against an `examples.Doc` schema returning one hit.
Caller is `alice@acme.com`; query matches via the BM25 + vector blend.
Tests the full happy-path envelope unwrap plus receipt linkage.
