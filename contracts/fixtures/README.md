# `contracts/fixtures/`

Wire fixtures consumed by every SealStack SDK test suite. Each scenario
is a directory containing three files:

- `description.md` — required, ~3 lines: what scenario, why.
- `request.json` — `{ method, path, headers, body }` of the request the
  SDK should send.
- `response.json` — `{ status, headers, body }` of the response the
  gateway returns.

## Naming

`<endpoint-or-namespace>-<outcome>` — e.g. `query-success`,
`register-schema-conflict`, `apply-ddl-validation-error`.

## Coverage

Every endpoint has a happy-path fixture. Every error class in the
taxonomy has at least one fixture from at least one endpoint that
surfaces it. See SDK contract spec §12.3.

## Cross-language parity

Every fixture in this directory must be consumed by both SDKs.
CI fails on coverage asymmetry — see each SDK's
`tests/.../corpus_coverage.*`.

## Regenerating

Fixtures are emitted by the live gateway, not hand-edited:

    cargo run --bin emit-fixtures -p sealstack-api-types

Nightly CI re-runs the emitter against the latest gateway and diffs
against the checked-in corpus to catch historic-corpus staleness.
