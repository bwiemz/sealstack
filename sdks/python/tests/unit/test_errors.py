import pytest
from sealstack.errors import (
    SealStackError, NotFoundError, UnknownSchemaError,
    UnauthorizedError, PolicyDeniedError, InvalidArgumentError,
    RateLimitedError, BackendError, from_wire_error,
)


def test_not_found_extends_sealstack_error():
    e = NotFoundError("missing", resource="schema:Foo")
    assert isinstance(e, SealStackError)
    assert e.resource == "schema:Foo"


def test_unknown_schema_extends_not_found():
    e = UnknownSchemaError("no such schema", schema="examples.Foo")
    assert isinstance(e, NotFoundError)
    assert isinstance(e, SealStackError)
    assert e.schema == "examples.Foo"


def test_policy_denied_carries_predicate():
    e = PolicyDeniedError("denied", predicate="rule.admin_only")
    assert e.predicate == "rule.admin_only"


def test_rate_limited_retry_after_optional():
    assert RateLimitedError("slow down", retry_after=None).retry_after is None
    assert RateLimitedError("slow down", retry_after=60).retry_after == 60


def test_backend_request_id_required():
    e = BackendError("kaboom", request_id="req-abc")
    assert e.request_id == "req-abc"


@pytest.mark.parametrize("code,klass", [
    ("not_found", NotFoundError),
    ("unknown_schema", UnknownSchemaError),
    ("unauthorized", UnauthorizedError),
    ("policy_denied", PolicyDeniedError),
    ("invalid_argument", InvalidArgumentError),
    ("rate_limited", RateLimitedError),
    ("backend", BackendError),
])
def test_from_wire_error_dispatch(code, klass):
    e = from_wire_error(
        {"code": code, "message": "msg"},
        headers={"x-request-id": "req-1", "retry-after": "30"},
    )
    assert isinstance(e, klass)


def test_from_wire_error_unknown_code_falls_back_to_backend():
    e = from_wire_error(
        {"code": "made_up", "message": "unknown"},
        headers={"x-request-id": "req-1"},
    )
    assert isinstance(e, BackendError)
    assert "made_up" in str(e)
