# `sealstack-api-types`

Wire types for the SealStack gateway REST API.

This crate is the **source of truth** for the JSON shapes the gateway
exchanges with its clients. It derives `JsonSchema` so the `emit-schemas`
binary produces JSON Schema artifacts that drive the TypeScript and
Python SDK codegen pipelines.

## Regenerating schemas

    cargo run --bin emit-schemas -p sealstack-api-types

Output goes to `schemas/`. CI verifies the output matches the checked-in
copy (regenerate-and-diff pattern).

## Why a separate crate

See [`docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md`](../../docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md) §6.
