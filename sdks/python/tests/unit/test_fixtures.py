"""Parametrized fixture-driven test mirroring corpus.test.ts.

Iterates contracts/fixtures/, dispatches each fixture through the
matching SDK call, and verifies the SDK either round-trips the
recorded body (success) or surfaces the wire error as a typed
exception (failure). Adding a fixture without registering a
DISPATCH entry fails the test with a clear message, so the corpus
and the test cannot drift silently.
"""

import json
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any
from urllib.parse import unquote

import httpx
import pytest
import respx

from sealstack import SealStack, SealStackError

# msw + tough-cookie reject "test"/"example" public suffixes; use a
# non-public host. Matches the TS tests so both languages talk to
# the same fake host.
HOST = "http://localhost.sealstack.local"

FIXTURES_ROOT = Path(__file__).resolve().parents[4] / "contracts" / "fixtures"

DispatchFn = Callable[[SealStack, dict[str, Any]], Awaitable[Any]]


def _path_id(req: dict[str, Any], idx: int) -> str:
    """Pull a percent-decoded path segment from a recorded request."""
    return unquote(req["path"].split("/")[idx])


async def _query_success(c: SealStack, r: dict[str, Any]) -> Any:
    body = r["body"]
    return await c.query(
        schema=body["schema"], query=body["query"], top_k=body.get("top_k"),
    )


async def _query_denied(c: SealStack, r: dict[str, Any]) -> Any:
    body = r["body"]
    return await c.query(schema=body["schema"], query=body["query"])


async def _register_schema(c: SealStack, r: dict[str, Any]) -> Any:
    return await c.admin.schemas.register(meta=r["body"]["meta"])


async def _apply_ddl(c: SealStack, r: dict[str, Any]) -> Any:
    qualified = _path_id(r, 3)
    return await c.admin.schemas.apply_ddl(qualified, ddl=r["body"]["ddl"])


async def _register_connector(c: SealStack, r: dict[str, Any]) -> Any:
    body = r["body"]
    return await c.admin.connectors.register(
        kind=body["kind"], schema=body["schema"], config=body["config"],
    )


async def _sync_connector(c: SealStack, r: dict[str, Any]) -> Any:
    return await c.admin.connectors.sync(_path_id(r, 3))


async def _list_schemas(c: SealStack, _r: dict[str, Any]) -> Any:
    return await c.schemas.list()


async def _list_connectors(c: SealStack, _r: dict[str, Any]) -> Any:
    return await c.connectors.list()


async def _get_receipt(c: SealStack, r: dict[str, Any]) -> Any:
    return await c.receipts.get(_path_id(r, 3))


async def _healthz(c: SealStack, _r: dict[str, Any]) -> Any:
    return await c.healthz()


DISPATCH: dict[str, DispatchFn] = {
    "query-success": _query_success,
    "query-policy-denied": _query_denied,
    "register-schema-success": _register_schema,
    "apply-ddl-success": _apply_ddl,
    "apply-ddl-validation-error": _apply_ddl,
    "register-connector-success": _register_connector,
    "sync-connector-success": _sync_connector,
    "list-schemas-success": _list_schemas,
    "list-connectors-success": _list_connectors,
    "get-receipt-not-found": _get_receipt,
    "healthz-success": _healthz,
}


def _all_fixture_names() -> list[str]:
    return sorted(p.name for p in FIXTURES_ROOT.iterdir() if p.is_dir())


@pytest.mark.parametrize("name", _all_fixture_names())
@respx.mock
async def test_fixture_round_trip(name: str) -> None:
    if name not in DISPATCH:
        raise AssertionError(
            f"no DISPATCH entry for fixture {name!r}; add one in test_fixtures.py",
        )

    fixture_dir = FIXTURES_ROOT / name
    req = json.loads((fixture_dir / "request.json").read_text())
    res = json.loads((fixture_dir / "response.json").read_text())

    method = req["method"]
    url = f"{HOST}{req['path']}"
    if method == "GET":
        route = respx.get(url)
    elif method == "POST":
        route = respx.post(url)
    else:
        raise NotImplementedError(f"unsupported method: {method}")

    def handler(request: httpx.Request) -> httpx.Response:
        if req["body"] is not None and method != "GET":
            assert json.loads(request.content) == req["body"]
        return httpx.Response(
            res["status"], headers=res["headers"], json=res["body"],
        )

    route.side_effect = handler

    async with SealStack.bearer(url=HOST, token="test-token") as client:
        dispatch = DISPATCH[name]
        if res["status"] >= 400:
            with pytest.raises(SealStackError):
                await dispatch(client, req)
        else:
            await dispatch(client, req)
