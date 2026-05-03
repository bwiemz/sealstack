# `contracts/` — SealStack API contract layer

This directory is the **language-agnostic, hand-written canonical**
layer of the SealStack API. It pairs with the generated wire types in
[`crates/sealstack-api-types/`](../crates/sealstack-api-types/) to
form the full SDK contract.

## Structure

- `fixtures/` — request/response pairs per scenario, consumed by every
  language SDK's test suite. See `fixtures/README.md`.
- `CHANGELOG.md` — wire-shape changes that affect any SDK.
- `COMPATIBILITY.md` — SDK-version × gateway-version compatibility matrix.

The full SDK contract lives at
[`docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md`](../docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md).
