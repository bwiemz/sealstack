import asyncio
import pytest
import respx
import httpx
from sealstack._http import HttpClient, HttpClientOptions, REDACTED_HEADERS
from sealstack.errors import BackendError


def make_client(**overrides) -> HttpClient:
    opts = HttpClientOptions(
        base_url="http://test",
        headers={},
        timeout_s=5.0,
        retry_attempts=0,
        retry_initial_backoff_ms=100,
    )
    for k, v in overrides.items():
        setattr(opts, k, v)
    return HttpClient(opts)


@respx.mock
async def test_returns_data_on_200():
    respx.get("http://test/x").respond(json={"data": {"ok": True}, "error": None})
    c = make_client()
    out = await c.request("GET", "/x")
    assert out == {"ok": True}


@respx.mock
async def test_throws_backend_error_on_500():
    respx.get("http://test/x").respond(
        status_code=500,
        headers={"x-request-id": "req-7"},
        json={"data": None, "error": {"code": "backend", "message": "boom"}},
    )
    c = make_client()
    with pytest.raises(BackendError):
        await c.request("GET", "/x")


@respx.mock
async def test_retries_5xx_then_succeeds():
    route = respx.get("http://test/x")
    route.side_effect = [
        httpx.Response(503),
        httpx.Response(503),
        httpx.Response(200, json={"data": {"ok": True}, "error": None}),
    ]
    c = make_client(retry_attempts=2, retry_initial_backoff_ms=5)
    out = await c.request("GET", "/x")
    assert out == {"ok": True}


@respx.mock
async def test_retries_429_with_retry_after():
    route = respx.get("http://test/x")
    route.side_effect = [
        httpx.Response(429, headers={"retry-after": "0"}),
        httpx.Response(200, json={"data": {"ok": True}, "error": None}),
    ]
    c = make_client(retry_attempts=1, retry_initial_backoff_ms=5)
    out = await c.request("GET", "/x")
    assert out == {"ok": True}


@respx.mock
async def test_cancellation_propagates_through_retry_sleep():
    respx.get("http://test/x").respond(503)
    c = make_client(retry_attempts=5, retry_initial_backoff_ms=200)

    async def driver():
        return await c.request("GET", "/x")

    task = asyncio.create_task(driver())
    await asyncio.sleep(0.05)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task


def test_redaction_list_includes_known_secret_headers():
    expected = {
        "authorization", "cookie", "x-api-key",
        "x-sealstack-user", "x-sealstack-tenant", "x-sealstack-roles",
        "x-cfg-user", "x-cfg-tenant", "x-cfg-roles",
    }
    assert REDACTED_HEADERS == expected


def test_debug_logs_redact_authorization():
    log: list[str] = []
    c = make_client(headers={"authorization": "Bearer secret"}, debug=log.append)
    c._log_request_for_test("GET", "/x")
    joined = "\n".join(log)
    assert "<redacted>" in joined
    assert "secret" not in joined
