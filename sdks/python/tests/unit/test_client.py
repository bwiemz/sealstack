import pytest
import warnings
from sealstack import SealStack


def test_bearer_factory_accepts_string_token():
    c = SealStack.bearer(url="http://localhost:7070", token="abc")
    assert c is not None


def test_bearer_factory_accepts_callable_token():
    c = SealStack.bearer(url="http://localhost:7070", token=lambda: "abc")
    assert c is not None


def test_unauthenticated_factory_requires_tenant():
    with pytest.raises(TypeError):
        # pyright: ignore - intentional missing kwarg
        SealStack.unauthenticated(url="http://localhost:7070", user="alice")


def test_unauthenticated_warns_for_non_local_url():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        SealStack.unauthenticated(
            url="https://gateway.acme.com",
            user="alice", tenant="default",
        )
        assert any("non-local" in str(x.message).lower() for x in w)


def test_unauthenticated_does_not_warn_for_localhost():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        SealStack.unauthenticated(
            url="http://localhost:7070",
            user="alice", tenant="default",
        )
        assert len(w) == 0


def test_exposes_namespaces():
    c = SealStack.bearer(url="http://localhost:7070", token="abc")
    assert c.schemas is not None
    assert c.connectors is not None
    assert c.receipts is not None
    assert c.admin is not None
    assert c.admin.schemas is not None
    assert c.admin.connectors is not None
