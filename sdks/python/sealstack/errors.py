"""Typed exception hierarchy for the SealStack SDK.

Mirrors the TS SDK's `class extends Error` shape. Flat hierarchy:
SealStackError base + one subclass per wire `code` (with
UnknownSchemaError as a NotFoundError subclass per spec §8).
"""

from __future__ import annotations


class SealStackError(Exception):
    """Base for every typed SDK error."""


class NotFoundError(SealStackError):
    def __init__(self, message: str, *, resource: str) -> None:
        super().__init__(message)
        self.resource = resource


class UnknownSchemaError(NotFoundError):
    def __init__(self, message: str, *, schema: str) -> None:
        super().__init__(message, resource=f"schema:{schema}")
        self.schema = schema


class UnauthorizedError(SealStackError):
    def __init__(self, message: str, *, realm: str | None = None) -> None:
        super().__init__(message)
        self.realm = realm


class PolicyDeniedError(SealStackError):
    def __init__(self, message: str, *, predicate: str) -> None:
        super().__init__(message)
        self.predicate = predicate


class InvalidArgumentError(SealStackError):
    def __init__(self, message: str, *, reason: str, field: str | None = None) -> None:
        super().__init__(message)
        self.reason = reason
        self.field = field


class RateLimitedError(SealStackError):
    def __init__(self, message: str, *, retry_after: int | None) -> None:
        super().__init__(message)
        self.retry_after = retry_after


class BackendError(SealStackError):
    def __init__(self, message: str, *, request_id: str) -> None:
        super().__init__(message)
        self.request_id = request_id


def from_wire_error(
    wire: dict[str, str], *, headers: dict[str, str]
) -> SealStackError:
    """Dispatch a wire error envelope to the right typed class.

    Unknown codes fall through to BackendError per spec §8.2.
    """
    code = wire.get("code", "")
    message = wire.get("message", "")
    request_id = headers.get("x-request-id", "unknown")
    retry_after_raw = headers.get("retry-after")
    retry_after: int | None = int(retry_after_raw) if retry_after_raw else None

    match code:
        case "not_found":
            return NotFoundError(message, resource="<unspecified>")
        case "unknown_schema":
            return UnknownSchemaError(message, schema="<unspecified>")
        case "unauthorized":
            return UnauthorizedError(message)
        case "policy_denied":
            return PolicyDeniedError(message, predicate="<unspecified>")
        case "invalid_argument":
            return InvalidArgumentError(message, reason=message)
        case "rate_limited":
            return RateLimitedError(message, retry_after=retry_after)
        case "backend":
            return BackendError(message, request_id=request_id)
        case _:
            return BackendError(
                f"unknown error code: {code} ({message})", request_id=request_id
            )
