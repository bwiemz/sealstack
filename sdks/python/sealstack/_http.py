"""Internal async HTTP client with retry, full jitter, and redaction.

Mirrors the TS SDK's HttpClient one-for-one. async-first; the sync
facade lives in `client.py` and wraps each async method with
`asyncio.run`.
"""

from __future__ import annotations

import asyncio
import random
from dataclasses import dataclass
from typing import Any, Callable

import httpx

from .errors import (
    BackendError,
    RateLimitedError,
    from_wire_error,
)

REDACTED_HEADERS = frozenset({
    "authorization",
    "cookie",
    "x-api-key",
    "x-sealstack-user", "x-sealstack-tenant", "x-sealstack-roles",
    "x-cfg-user", "x-cfg-tenant", "x-cfg-roles",
})


@dataclass
class HttpClientOptions:
    base_url: str
    headers: dict[str, str]
    timeout_s: float
    retry_attempts: int
    retry_initial_backoff_ms: int
    debug: Callable[[str], None] | None = None


class HttpClient:
    def __init__(self, opts: HttpClientOptions) -> None:
        self._opts = opts
        self._client = httpx.AsyncClient(
            base_url=opts.base_url.rstrip("/"),
            headers=opts.headers,
            timeout=opts.timeout_s,
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    def _log_request_for_test(self, method: str, path: str) -> None:
        self._log_request(method, path)

    async def request(
        self,
        method: str,
        path: str,
        *,
        body: Any = None,
        no_retry: bool = False,
        timeout_s: float | None = None,
    ) -> Any:
        max_attempts = 1 if no_retry else self._opts.retry_attempts + 1
        last_error: Exception | None = None

        for attempt in range(1, max_attempts + 1):
            try:
                return await self._attempt(method, path, body, timeout_s)
            except Exception as e:
                last_error = e
                if not self._should_retry(e, attempt, max_attempts):
                    raise
                await self._sleep(self._backoff_ms(attempt, e) / 1000.0)

        assert last_error is not None
        raise last_error

    async def _attempt(
        self, method: str, path: str, body: Any, timeout_s: float | None
    ) -> Any:
        self._log_request(method, path)
        timeout = timeout_s if timeout_s is not None else self._opts.timeout_s
        resp = await self._client.request(
            method, path, json=body, timeout=timeout
        )
        headers = {k.lower(): v for k, v in resp.headers.items()}
        env = resp.json() if resp.content else {"data": None, "error": None}

        if resp.status_code >= 400 or env.get("error"):
            self._log_error_response(resp.status_code, headers, resp.text)
            wire = env.get("error") or {"code": "backend", "message": f"HTTP {resp.status_code}"}
            raise from_wire_error(wire, headers=headers)
        if env.get("data") is None:
            raise BackendError(
                "response envelope missing data",
                request_id=headers.get("x-request-id", "unknown"),
            )
        return env["data"]

    def _should_retry(self, e: Exception, attempt: int, max_attempts: int) -> bool:
        if attempt >= max_attempts:
            return False
        if isinstance(e, (RateLimitedError, BackendError)):
            return True
        if isinstance(e, httpx.TransportError):  # network errors
            return True
        return False

    def _backoff_ms(self, attempt: int, e: Exception) -> float:
        if isinstance(e, RateLimitedError) and e.retry_after is not None and e.retry_after >= 0:
            return e.retry_after * 1000.0
        base = self._opts.retry_initial_backoff_ms * (2 ** (attempt - 1))
        # Full jitter: uniform random in [0, base * 1.25].
        return random.uniform(0.0, base * 1.25)

    async def _sleep(self, seconds: float) -> None:
        # asyncio.sleep is naturally cancellable; cancellation propagates
        # through the retry loop without explicit handling.
        await asyncio.sleep(seconds)

    def _log_request(self, method: str, path: str) -> None:
        if self._opts.debug is None:
            return
        redacted = self._redact_headers(self._opts.headers)
        self._opts.debug(f"→ {method} {path} headers={redacted}")

    def _log_error_response(
        self, status: int, headers: dict[str, str], body: str
    ) -> None:
        if self._opts.debug is None:
            return
        self._opts.debug(
            f"← {status} headers={self._redact_headers(headers)} body={body}"
        )

    @staticmethod
    def _redact_headers(h: dict[str, str]) -> dict[str, str]:
        return {
            k.lower(): ("<redacted>" if k.lower() in REDACTED_HEADERS else v)
            for k, v in h.items()
        }
