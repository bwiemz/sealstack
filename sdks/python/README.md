# `sealstack` — Python SDK

The Python SDK for [SealStack](https://github.com/bwiemz/sealstack).

For the canonical contract, see
[the SDK design spec](https://github.com/bwiemz/sealstack/blob/main/docs/superpowers/specs/2026-05-02-sdk-clients-typescript-python-design.md).

## Install

    pip install sealstack

## Quickstart

```python
"""SealStack Python SDK Quickstart."""

import asyncio
from sealstack import SealStack


async def main():
    async with SealStack.bearer(url="http://localhost:7070", token="dev-token") as client:
        await client.admin.schemas.register(meta={"...": "compiled schema"})
        await client.admin.schemas.apply_ddl("examples.Doc", ddl="/* ddl */")
        await client.admin.connectors.register(
            kind="local-files", schema="examples.Doc", config={"root": "./docs"},
        )
        await client.admin.connectors.sync("local-files/examples.Doc")
        result = await client.query(schema="examples.Doc", query="getting started")
        print(result)


asyncio.run(main())
```

See [`examples/quickstart.py`](./examples/quickstart.py) for a runnable copy.
The CI gate `verify-readme-quickstart.sh` enforces that the code block above
matches the example file byte-for-byte.

## Layout

- `sealstack/client.py` — the `SealStack` class plus `bearer()` and
  `unauthenticated()` factories, the async context-manager protocol, and
  the `sync()` facade.
- `sealstack/namespaces/` — `schemas`, `connectors`, `receipts` (read), and
  `admin` (write).
- `sealstack/errors.py` — typed error hierarchy (`SealStackError` plus seven
  subclasses) returned to callers.
- `sealstack/_generated/` — JSON-schema-derived pydantic models; do not
  hand-edit.
