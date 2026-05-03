"""Schemas namespace (read-only)."""

from typing import Any

from .._http import HttpClient


class SchemasNamespace:
    def __init__(self, http: HttpClient) -> None:
        self._http = http

    async def list(self) -> list[Any]:
        out = await self._http.request("GET", "/v1/schemas")
        return out["schemas"]

    async def get(self, qualified: str) -> Any:
        return await self._http.request("GET", f"/v1/schemas/{qualified}")
