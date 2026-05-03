"""Smoke suite: hits a live gateway on SEALSTACK_GATEWAY_URL.

Cases are intentionally bounded to operations that work against a
fresh (no-seed-data) gateway: health endpoints, registering a
throwaway schema, and exercising one negative path (404 receipt).
Cases that need a populated corpus belong to a heavier integration
harness.

Skipped wholesale when SEALSTACK_GATEWAY_URL is unset, so the file
is safe to leave wired into CI paths that don't yet provision a
gateway.
"""

import os
import time

import pytest

from sealstack import NotFoundError, SealStack

URL = os.environ.get("SEALSTACK_GATEWAY_URL")

pytestmark = pytest.mark.skipif(
    not URL, reason="SEALSTACK_GATEWAY_URL not set; skipping live-gateway smoke",
)


async def test_healthz_reports_ok():
    async with SealStack.bearer(url=URL, token="test-token") as c:
        out = await c.healthz()
    assert out == {"status": "ok"}


async def test_readyz_reports_ok():
    async with SealStack.bearer(url=URL, token="test-token") as c:
        out = await c.readyz()
    assert out == {"status": "ok"}


async def test_register_schema_returns_qualified():
    ns = f"smoke_py_{int(time.time() * 1000)}"
    async with SealStack.bearer(url=URL, token="test-token") as c:
        out = await c.admin.schemas.register(meta={
            "namespace": ns,
            "name": "Doc",
            "version": 1,
            "primary_key": "id",
            "fields": [
                {"name": "id",   "ty": "string", "nullable": False},
                {"name": "body", "ty": "string", "nullable": False},
            ],
            "table": f"{ns}_doc_v1",
            "collection": f"{ns}_doc_v1",
            "hybrid_alpha": 0.5,
        })
    assert out["qualified"] == f"{ns}.Doc"


async def test_list_schemas_returns_array():
    async with SealStack.bearer(url=URL, token="test-token") as c:
        out = await c.schemas.list()
    assert isinstance(out, list)


async def test_unknown_receipt_raises_not_found():
    async with SealStack.bearer(url=URL, token="test-token") as c:
        with pytest.raises(NotFoundError):
            await c.receipts.get("01JD0BOGUS00000000000000000")
