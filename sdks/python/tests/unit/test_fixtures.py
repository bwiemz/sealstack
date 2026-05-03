import json
from pathlib import Path

import httpx
import respx

from sealstack import SealStack

# msw + tough-cookie equivalent isn't an issue with respx, but use a
# non-public-suffix host anyway for parity with the TS SDK's tests.
HOST = "http://localhost.sealstack.local"

FIXTURE_ROOT = (
    Path(__file__).resolve().parents[4] / "contracts" / "fixtures" / "query-success"
)


@respx.mock
async def test_query_success_fixture():
    req = json.loads((FIXTURE_ROOT / "request.json").read_text())
    res = json.loads((FIXTURE_ROOT / "response.json").read_text())

    route = respx.post(f"{HOST}{req['path']}")

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        assert body == req["body"]
        return httpx.Response(
            res["status"], headers=res["headers"], json=res["body"]
        )

    route.side_effect = handler

    async with SealStack.bearer(url=HOST, token="test-token") as client:
        out = await client.query(
            schema=req["body"]["schema"],
            query=req["body"]["query"],
            top_k=req["body"]["top_k"],
        )
    assert out == res["body"]["data"]
