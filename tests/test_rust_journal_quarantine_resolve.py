"""`ui.journals.quarantine.resolve` against the unchanged Python daemon.

One audited price override or exclusion, the optional rebuild and every
argument refusal must match Python's envelope and leave the same book. The
metadata service draws its audit event id and edit time from the system
clock in both runtimes' metadata layer, so only those generated identities
and edit timestamps are masked; every other stored value is compared.
"""
import json
import re
import shutil

import pytest

from kassiber.db import open_db
from tests.test_rust_journal_commands import BINARY, KEY, book_dump, invoke, seed
from tests.test_rust_journal_commands import environment  # noqa: F401
from tests.test_rust_journal_views import Reads, normalized

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")
KIND = "ui.journals.quarantine.resolve"
UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
NOW = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z")
EDIT_TABLES = ("transaction_edit_events", "transaction_edit_fields")


def resolve(root, env, native, cipher, requests):
    with Reads(root, env, native=native) as daemon:
        daemon.unlock(cipher)
        rows = []
        for number, args in enumerate(requests):
            daemon.send(KIND, f"r{number}", args)
            rows.append(daemon.until(f"r{number}", KIND))
        daemon.send("daemon.shutdown", "stop")
        daemon.until("stop")
        daemon.process.stdin.close()
        daemon.process.wait(timeout=60)
        assert daemon.process.returncode == 0
    return rows


def masked(value):
    """Mask generated audit identities and edit times, recursively."""
    if isinstance(value, str):
        return NOW.sub("<now>", UUID.sub("<uuid>", value))
    if isinstance(value, list):
        return [masked(item) for item in value]
    if isinstance(value, dict):
        return {key: masked(item) for key, item in value.items()}
    return value


def book(root, cipher):
    tables = book_dump(root, cipher)
    result = {}
    for name, table in tables.items():
        rows = table["rows"]
        if name in EDIT_TABLES or name.startswith("journal_") or name == "transactions":
            rows = [masked(row) for row in rows]
            if name == "transactions":
                # The override stamps its capture time from the system clock.
                index = table["columns"].index("pricing_fetched_at")
                rows = [[*row[:index], "<now>" if row[index] else row[index], *row[index + 1:]] for row in rows]
        result[name] = dict(columns=table["columns"], rows=sorted(rows, key=lambda r: json.dumps(r, sort_keys=True)))
    return result


def compare(tmp_path, environment, root, requests, cipher=False):
    native = tmp_path / "rust"
    shutil.copytree(root, native)
    expected = resolve(root, environment, False, cipher, requests)
    actual = resolve(native, environment, True, cipher, requests)
    (tmp_path / "resolve.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=1))
    for args, got, want in zip(requests, actual, normalized(expected)):
        assert json.dumps(masked(got)) == json.dumps(masked(want)), (args, got, want)
    assert book(native, cipher) == book(root, cipher)
    return actual


def quarantined(tmp_path, environment, cipher=False):
    root = seed(tmp_path / "python", cipher=cipher, shape="blocked")
    assert invoke(root, environment, cipher=cipher).returncode == 0
    with open_db(root, passphrase=KEY if cipher else None) as conn:
        assert conn.execute("SELECT COUNT(*) FROM journal_quarantines").fetchone()[0] > 0
        conn.execute("UPDATE transactions SET external_id='ext-a' WHERE id='a'")
    conn.close()
    return root


REFUSALS = [
    None, [], {}, {"bogus": 1, "transaction": "a"}, {"transaction": "  ", "action": "exclude", "reason": "r"},
    {"transaction": 7, "action": "exclude", "reason": "r"}, {"transaction": "a", "action": "delete", "reason": "r"},
    {"transaction": "a", "action": ["exclude"], "reason": "r"}, {"transaction": "a", "action": "exclude"},
    {"transaction": "a", "action": "exclude", "reason": "   "},
    {"transaction": "a", "action": "exclude", "reason": "r", "reprocess": 1},
    {"transaction": "a", "action": "price_override", "reason": "r"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": 1, "fiat_value": 2},
    {"transaction": "a", "action": "exclude", "reason": "r", "fiat_value": 2},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "abc"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "-1"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "0"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": ""},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": "NaN"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_value": "Infinity"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_value": "-5"},
    {"transaction": "a", "action": "price_override", "reason": "r", "fiat_rate": True},
    {"transaction": "b", "action": "exclude", "reason": "r"},
    {"transaction": "missing", "action": "exclude", "reason": "r"},
]


@pytest.mark.parametrize("cipher", [False, True])
def test_quarantine_resolve_refusals_match_python_without_writes(tmp_path, environment, cipher):
    root = quarantined(tmp_path, environment, cipher)
    before = book(root, cipher)
    rows = compare(tmp_path, environment, root, REFUSALS, cipher)
    assert all(row["kind"] == "error" for row in rows)
    assert book(tmp_path / "rust", cipher) == before


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("reprocess", [True, False])
@pytest.mark.parametrize("request_args", [
    {"action": "price_override", "fiat_rate": "41234.5"},
    {"action": "price_override", "fiat_rate": 41234.5},
    {"action": "price_override", "fiat_value": "4123.45678901234567890123456789"},
    {"action": "exclude"},
], ids=["rate", "float-rate", "value", "exclude"])
def test_quarantine_resolve_matches_python_envelope_and_book(tmp_path, environment, cipher, reprocess, request_args):
    root = quarantined(tmp_path, environment, cipher)
    args = {"transaction": " ext-a ", "reason": " reviewed receipt ", "reprocess": reprocess, **request_args}
    rows = compare(tmp_path, environment, root, [args], cipher)
    assert rows[0]["kind"] == KIND, rows
    assert rows[0]["data"]["reprocessed"] is reprocess
