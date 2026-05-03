"""Connectors namespace (read-only)."""

from typing import Any

from .._http import HttpClient


class ConnectorsNamespace:
    def __init__(self, http: HttpClient) -> None:
        self._http = http

    async def list(self) -> list[Any]:
        out = await self._http.request("GET", "/v1/connectors")
        return out["connectors"]
