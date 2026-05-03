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
