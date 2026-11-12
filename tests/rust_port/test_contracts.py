"""Pinned Python contracts for the native backend port."""

import ast
from pathlib import Path

import pytest

from tests.rust_port import contracts


# One snapshot per test id, so xdist builds them in parallel.
@pytest.mark.parametrize("name", contracts.SNAPSHOT_NAMES)
def test_wire_and_source_contracts_match_the_reviewed_snapshots(name):
    content = contracts.snapshot(name)
    assert (contracts.FIXTURES / name).read_bytes() == content, (
        f"{name} drifted; review the change, then run "
        "uv run --locked python -m tests.rust_port.contracts update"
    )


def test_error_index_keeps_positional_wrappers_and_dynamic_retry_flags(monkeypatch):
    source = '''
def fail(message, code="wrapped"):
    raise AppError(message, code=code, retryable=True)
fail("bad", "positional")
AppError("busy", code="database_busy", retryable=can_retry)
record = {"code": "notice"}
'''
    monkeypatch.setattr(contracts, "sources", lambda: [(Path("kassiber/example.py"), ast.parse(source))])
    index = contracts.error_codes()
    positional = next(site for site in index["sites"] if site["code"] == "positional")
    assert positional["retryable"] is True
    dynamic = next(site for site in index["sites"] if site["code"] == "database_busy")
    assert dynamic["retryable"] == {"expression": "can_retry"}
    record = next(site for site in index["sites"] if site["code"] == "notice")
    assert record["retryable"] is None
    assert index["defaults"][0]["code"] == "wrapped"
