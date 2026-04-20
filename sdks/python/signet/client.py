"""Thin httpx wrapper around the Signet REST API."""

from __future__ import annotations

from typing import Any

import httpx
from pydantic import BaseModel


class ContextQuery(BaseModel):
    query: str
    top_k: int | None = None
    filters: dict[str, Any] | None = None


class ContextChunk(BaseModel):
    id: str
    content: str
    score: float
    metadata: dict[str, Any] = {}


class Receipt(BaseModel):
    id: str
    sources: list[dict[str, Any]] = []


class ContextResponse(BaseModel):
    chunks: list[ContextChunk]
    receipt: Receipt


class Schema(BaseModel):
    name: str
    version: int
    namespace: str | None = None


class Connector(BaseModel):
    id: str
    name: str
    enabled: bool


class SignetClient:
    def __init__(self, base_url: str, token: str | None = None) -> None:
        self.base_url = base_url.rstrip("/")
        self._headers = {"Content-Type": "application/json"}
        if token:
            self._headers["Authorization"] = f"Bearer {token}"
        self._client = httpx.Client(headers=self._headers, timeout=30.0)

    def query(self, q: ContextQuery) -> ContextResponse:
        r = self._client.post(f"{self.base_url}/v1/query", json=q.model_dump(exclude_none=True))
        r.raise_for_status()
        return ContextResponse.model_validate(r.json())

    def list_schemas(self) -> list[Schema]:
        r = self._client.get(f"{self.base_url}/v1/schemas")
        r.raise_for_status()
        return [Schema.model_validate(s) for s in r.json()]

    def get_schema(self, name: str) -> Schema:
        r = self._client.get(f"{self.base_url}/v1/schemas/{name}")
        r.raise_for_status()
        return Schema.model_validate(r.json())

    def list_connectors(self) -> list[Connector]:
        r = self._client.get(f"{self.base_url}/v1/connectors")
        r.raise_for_status()
        return [Connector.model_validate(c) for c in r.json()]

    def sync_connector(self, connector_id: str) -> dict[str, Any]:
        r = self._client.post(f"{self.base_url}/v1/connectors/{connector_id}/sync")
        r.raise_for_status()
        return r.json()

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> SignetClient:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()
