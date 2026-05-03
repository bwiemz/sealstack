from pathlib import Path

# Fixtures consumed by the Python SDK's tests. Update as you add tests.
PY_CONSUMED_FIXTURES = {
    "query-success",
}


def test_every_fixture_consumed_by_py_sdk():
    root = Path(__file__).resolve().parents[4] / "contracts" / "fixtures"
    all_fixtures = {p.name for p in root.iterdir() if p.is_dir()}
    missing = all_fixtures - PY_CONSUMED_FIXTURES
    assert missing == set(), f"unconsumed fixtures: {missing}"
