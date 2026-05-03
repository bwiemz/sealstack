# `@sealstack/client`

The TypeScript SDK for [SealStack](https://github.com/bwiemz/sealstack).

For the canonical contract, see
[the SDK design spec](https://github.com/bwiemz/sealstack/blob/main/docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md).

## Install

    pnpm add @sealstack/client

## Quickstart

```typescript
import { SealStack } from "@sealstack/client";

const client = SealStack.bearer({ url: "http://localhost:7070", token: "dev-token" });
await client.admin.schemas.register({ meta: { /* compiled schema */ } });
await client.admin.schemas.applyDdl("examples.Doc", { ddl: "/* ddl */" });
await client.admin.connectors.register({ kind: "local-files", schema: "examples.Doc", config: { root: "./docs" } });
await client.admin.connectors.sync("local-files/examples.Doc");
const result = await client.query({ schema: "examples.Doc", query: "getting started" });
console.log(result);
```

See [`examples/quickstart.ts`](./examples/quickstart.ts) for a runnable copy.
The CI gate `verify-readme-quickstart.sh` enforces that the code block above
matches the example file byte-for-byte.

## Layout

- `src/client.ts` — the `SealStack` class plus `bearer()` and
  `unauthenticated()` factories.
- `src/namespaces/` — `schemas`, `connectors`, `receipts` (read), and
  `admin` (write).
- `src/errors.ts` — typed error hierarchy (`SealStackError` plus seven
  subclasses) returned to callers.
- `src/generated/` — JSON-schema-derived `.d.ts` files; do not hand-edit.
