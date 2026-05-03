"""SealStack SDK entry point: factories + namespace dispatch."""

from __future__ import annotations

import os
import warnings
from typing import Any, Callable
from urllib.parse import urlparse

from ._http import HttpClient, HttpClientOptions
from .namespaces.admin import AdminNamespace
from .namespaces.connectors import ConnectorsNamespace
from .namespaces.receipts import ReceiptsNamespace
from .namespaces.schemas import SchemasNamespace

_LOCAL_HOSTS = {"localhost", "127.0.0.1", "host.docker.internal"}


def _looks_like_local(url: str) -> bool:
    try:
        host = urlparse(url).hostname or ""
    except Exception:
        return False
    return host in _LOCAL_HOSTS or host.endswith(".local")


class SealStack:
    """The SealStack client. Construct via `SealStack.bearer()` or
    `SealStack.unauthenticated()` — the constructor itself is private."""

    def __init__(self, http: HttpClient) -> None:
        self._http = http
        self.schemas = SchemasNamespace(http)
        self.connectors = ConnectorsNamespace(http)
        self.receipts = ReceiptsNamespace(http)
        self.admin = AdminNamespace(http)

    @classmethod
    def bearer(
        cls,
        *,
        url: str,
        token: str | Callable[[], str],
        timeout: float = 30.0,
        retry_attempts: int = 2,
        retry_initial_backoff_ms: int = 200,
        debug: bool | Callable[[str], None] = False,
    ) -> "SealStack":
        token_fn: Callable[[], str] = token if callable(token) else (lambda: token)
        headers = {"authorization": f"Bearer {token_fn()}"}
        return cls(_make_http(url, headers, timeout, retry_attempts,
                              retry_initial_backoff_ms, debug))

    @classmethod
    def unauthenticated(
        cls,
        *,
        url: str,
        user: str,
        tenant: str,
        roles: list[str] | None = None,
        timeout: float = 30.0,
        retry_attempts: int = 2,
        retry_initial_backoff_ms: int = 200,
        debug: bool | Callable[[str], None] = False,
    ) -> "SealStack":
        if not tenant:
            raise TypeError("SealStack.unauthenticated() requires `tenant`")

        if not _looks_like_local(url):
            warnings.warn(
                f"SealStack.unauthenticated() called against non-local URL {url}. "
                "Production gateways should reject these requests, but you should use "
                "bearer() in any code that runs outside your laptop.",
                stacklevel=2,
            )

        headers: dict[str, str] = {
            "x-sealstack-user": user,
            "x-sealstack-tenant": tenant,
        }
        if roles:
            headers["x-sealstack-roles"] = ",".join(roles)
        return cls(_make_http(url, headers, timeout, retry_attempts,
                              retry_initial_backoff_ms, debug))

    async def query(
        self,
        *,
        schema: str,
        query: str,
        top_k: int | None = None,
        filters: Any = None,
    ) -> Any:
        # Drop None-valued keys to match the TS SDK, where `undefined`
        # entries are stripped by JSON.stringify. Keeps the on-wire body
        # byte-equal to the recorded contract fixtures.
        body: dict[str, Any] = {"schema": schema, "query": query}
        if top_k is not None:
            body["top_k"] = top_k
        if filters is not None:
            body["filters"] = filters
        return await self._http.request("POST", "/v1/query", body=body)

    async def healthz(self) -> Any:
        return await self._http.request("GET", "/healthz")

    async def readyz(self) -> Any:
        return await self._http.request("GET", "/readyz")

    async def __aenter__(self) -> "SealStack":
        return self

    async def __aexit__(self, *_exc: object) -> None:
        await self._http.aclose()

    def sync(self) -> "SyncSealStack":
        return SyncSealStack(self)


class SyncSealStack:
    """Sync facade. Each method runs the async one to completion via
    asyncio.run. Public surface is identical."""

    def __init__(self, inner: SealStack) -> None:
        self._inner = inner

    def query(self, **kwargs: Any) -> Any:
        import asyncio
        return asyncio.run(self._inner.query(**kwargs))

    # Mirror the rest as needed; in v0.3 the canonical surface is async.

    def __enter__(self) -> "SyncSealStack":
        return self

    def __exit__(self, *_exc: object) -> None:
        import asyncio
        asyncio.run(self._inner._http.aclose())


def _make_http(
    url: str,
    headers: dict[str, str],
    timeout: float,
    retry_attempts: int,
    retry_initial_backoff_ms: int,
    debug: bool | Callable[[str], None],
) -> HttpClient:
    cb: Callable[[str], None] | None
    if debug is True:
        cb = lambda m: print(f"[sealstack] {m}")  # noqa: E731
    elif callable(debug):
        cb = debug
    elif os.environ.get("SEALSTACK_SDK_DEBUG") == "1":
        cb = lambda m: print(f"[sealstack] {m}")  # noqa: E731
    else:
        cb = None

    opts = HttpClientOptions(
        base_url=url,
        headers=headers,
        timeout_s=timeout,
        retry_attempts=retry_attempts,
        retry_initial_backoff_ms=retry_initial_backoff_ms,
        debug=cb,
    )
    return HttpClient(opts)
