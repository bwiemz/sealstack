"""Receipts namespace (read-only)."""

from typing import Any

from .._http import HttpClient


class ReceiptsNamespace:
    def __init__(self, http: HttpClient) -> None:
        self._http = http

    async def get(self, id: str) -> Any:
        return await self._http.request("GET", f"/v1/receipts/{id}")
