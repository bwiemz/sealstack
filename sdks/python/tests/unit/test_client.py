import warnings

import httpx
import pytest
import respx

from sealstack import SealStack

HOST = "http://localhost.sealstack.local"


def test_bearer_factory_accepts_string_token():
    c = SealStack.bearer(url="http://localhost:7070", token="abc")
    assert c is not None


def test_bearer_factory_accepts_callable_token():
    c = SealStack.bearer(url="http://localhost:7070", token=lambda: "abc")
    assert c is not None


def test_unauthenticated_factory_requires_tenant():
    with pytest.raises(TypeError):
        # pyright: ignore - intentional missing kwarg
        SealStack.unauthenticated(url="http://localhost:7070", user="alice")


def test_unauthenticated_warns_for_non_local_url():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        SealStack.unauthenticated(
            url="https://gateway.acme.com",
            user="alice", tenant="default",
        )
        assert any("non-local" in str(x.message).lower() for x in w)


def test_unauthenticated_does_not_warn_for_localhost():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        SealStack.unauthenticated(
            url="http://localhost:7070",
            user="alice", tenant="default",
        )
        assert len(w) == 0


def test_exposes_namespaces():
    c = SealStack.bearer(url="http://localhost:7070", token="abc")
    assert c.schemas is not None
    assert c.connectors is not None
    assert c.receipts is not None
    assert c.admin is not None
    assert c.admin.schemas is not None
    assert c.admin.connectors is not None


@respx.mock
async def test_token_factory_re_evaluates_per_request():
    """A `token=lambda: ...` rotates per-request, not just at construction."""
    state = {"n": 0}

    def token_fn() -> str:
        state["n"] += 1
        return f"t-{state['n']}"

    seen: list[str] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request.headers.get("authorization", ""))
        return httpx.Response(200, json={"data": {"status": "ok"}, "error": None})

    respx.get(f"{HOST}/healthz").side_effect = handler

    async with SealStack.bearer(url=HOST, token=token_fn) as c:
        await c.healthz()
        await c.healthz()
        await c.healthz()

    assert seen == ["Bearer t-1", "Bearer t-2", "Bearer t-3"]
