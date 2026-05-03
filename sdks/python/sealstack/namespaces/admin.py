"""Admin namespace: schema and connector management.

Per spec §9.2, admin operations do not auto-retry in v0.3.
"""

from typing import Any

from .._http import HttpClient


class _AdminSchemas:
    def __init__(self, http: HttpClient) -> None:
        self._http = http

    async def register(self, *, meta: Any) -> Any:
        return await self._http.request(
            "POST", "/v1/schemas",
            body={"meta": meta}, no_retry=True,
        )

    async def apply_ddl(self, qualified: str, *, ddl: str) -> Any:
        return await self._http.request(
            "POST", f"/v1/schemas/{qualified}/ddl",
            body={"ddl": ddl}, no_retry=True, timeout_s=60.0,
        )


class _AdminConnectors:
    def __init__(self, http: HttpClient) -> None:
        self._http = http

    async def register(
        self, *, kind: str, schema: str, config: Any
    ) -> Any:
        return await self._http.request(
            "POST", "/v1/connectors",
            body={"kind": kind, "schema": schema, "config": config},
            no_retry=True,
        )

    async def sync(self, id: str) -> Any:
        return await self._http.request(
            "POST", f"/v1/connectors/{id}/sync", no_retry=True,
        )


class AdminNamespace:
    def __init__(self, http: HttpClient) -> None:
        self.schemas = _AdminSchemas(http)
        self.connectors = _AdminConnectors(http)
