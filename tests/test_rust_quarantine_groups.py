"""Quarantine root causes (#681) on the Rust backend against unchanged Python.

`journals quarantine groups` must answer byte for byte as Python's CLI, the
`ui.journals.quarantine` summary must carry the same counters, currency and
definitions, and `ui.journals.quarantine.resolve` must resolve a `root:`
group's root, with and without a reprocess, reporting the same effect and
leaving the same book. Plaintext and SQLCipher.
"""
import base64
import json
import shutil
import subprocess
import sys

import pytest

from kassiber.db import open_db
from tests.test_rust_journal_commands import BINARY, KEY, ROOT, arguments, seed
from tests.test_rust_journal_commands import environment  # noqa: F401
from tests.test_rust_journal_quarantine_resolve import book as masked_book, masked, resolve
from tests.test_rust_journal_views import Reads, normalized

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")

# a is a root waiting rows name; b and s wait on it; i is downstream with no
# nameable root; t is a root of its own; b2 waits on i, a downstream row, so
# it resolves to nothing either.
QUARANTINES = [
    ("a", "missing_spot_price", {}),
    ("b", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["a"]}),
    ("s", "custody_basis_barrier", {"root_transaction_ids": ["a", "missing"]}),
    ("i", "basis_provenance_incomplete", {}),
    ("t", "insufficient_lots", {"asset": "BTC"}),
]


def held_book(root, cipher):
    seed(root, cipher=cipher)
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        conn.execute("DELETE FROM journal_quarantines")
        conn.executemany(
            "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at)"
            " VALUES(?,'w','p',?,?,'2024-05-01T00:00:00Z')",
            [(tx, reason, json.dumps(detail)) for tx, reason, detail in QUARANTINES],
        )
        conn.execute("UPDATE profiles SET fiat_currency='eur'")
        conn.commit()
    finally:
        conn.close()
    return root


def cli(root, env, cipher, native, extra):
    with arguments(root, cipher) as (args, fds):
        args = [*args[:-2], "journals", "quarantine", "groups", *extra]
        prefix = [str(BINARY)] if native else [sys.executable, "-m", "tests.rust_port.journal_cli_reference"]
        return subprocess.run([*prefix, *args], env=env, pass_fds=fds, capture_output=True, cwd=ROOT, timeout=120)


def same(reference, native, env, cipher, extra):
    expected = cli(reference, env, cipher, False, extra)
    actual = cli(native, env, cipher, True, extra)
    assert (actual.returncode, actual.stdout, actual.stderr) == (
        expected.returncode, expected.stdout, expected.stderr
    ), extra
    return json.loads(expected.stdout) if expected.stdout else None


def tamper(cursor, change):
    payload = json.loads(base64.urlsafe_b64decode(cursor))
    change(payload)
    return base64.urlsafe_b64encode(json.dumps(payload).encode()).decode()


@pytest.mark.parametrize("cipher", [False, True])
def test_groups_leaf_matches_python_byte_for_byte(tmp_path, environment, cipher):
    reference = held_book(tmp_path / "python", cipher)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    overview = same(reference, native, environment, cipher, [])
    data = overview["data"]
    assert data["count"] == 5 and data["waiting_count"] == 2
    assert [group["group_id"] for group in data["by_root"]] == ["root:a", "root:t"]
    assert data["by_root"][0]["dependent_count"] == 2
    assert [group["group_id"] for group in data["unrooted"]] == ["unrooted:basis_provenance_incomplete"]
    for extra in (
        ["--limit", "1"], ["--limit", "100"], ["--limit", "0"], ["--limit", "101"],
        ["--group-id", "root:a"], ["--group-id", "root:t"], ["--group-id", "root:b"],
        ["--group-id", "reason:missing_spot_price"], ["--group-id", "unrooted:basis_provenance_incomplete"],
        ["--group-id", "reason:none"], ["--group-id", "bogus"], ["--group-id", ""],
        ["--group-id", "root:a", "--limit", "200"], ["--group-id", "root:a", "--limit", "201"],
        ["--cursor", "abc"], ["--workspace", "missing"],
    ):
        same(reference, native, environment, cipher, extra)
    # Page through root:a one member at a time with Python's cursors.
    cursor, members = None, []
    while True:
        page = same(reference, native, environment, cipher,
                    ["--group-id", "root:a", "--limit", "1", *(["--cursor", cursor] if cursor else [])])
        members += [member["transaction_id"] for member in page["data"]["members"]]
        cursor = page["data"]["next_cursor"]
        if cursor is None:
            break
        for broken in (
            cursor[:-2], cursor + "!!", "e30=", base64.urlsafe_b64encode(b"[]").decode(),
            tamper(cursor, lambda p: p.update(after=[0, "x"])),
            tamper(cursor, lambda p: p.update(after=[True, "", ""])),
            tamper(cursor, lambda p: p["scope"].__setitem__(1, float(p["scope"][1]))),
            tamper(cursor, lambda p: p["scope"].__setitem__(4, "0" * 16)),
            tamper(cursor, lambda p: p.update(after=[-1, "", ""])),
        ):
            same(reference, native, environment, cipher, ["--group-id", "root:a", "--cursor", broken])
        same(reference, native, environment, cipher, ["--group-id", "root:t", "--cursor", cursor])
    assert members == ["a", "b", "s"]


def daemon_rows(root, env, native, cipher, requests):
    with Reads(root, env, native=native) as daemon:
        daemon.unlock(cipher)
        rows = []
        for number, (kind, args) in enumerate(requests):
            daemon.send(kind, f"r{number}", args)
            rows.append(daemon.until(f"r{number}", kind))
        daemon.send("daemon.shutdown", "stop")
        daemon.until("stop")
        daemon.process.stdin.close()
        daemon.process.wait(timeout=60)
        assert daemon.process.returncode == 0
    return rows


@pytest.mark.parametrize("cipher", [False, True])
def test_quarantine_summary_carries_currency_and_definitions(tmp_path, environment, cipher):
    reference = held_book(tmp_path / "python", cipher)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    requests = [("ui.journals.quarantine", args) for args in (
        {}, {"scope": "attention"}, {"scope": "waiting", "limit": 1}, {"offset": 3},
    )]
    expected = daemon_rows(reference, environment, False, cipher, requests)
    actual = daemon_rows(native, environment, True, cipher, requests)
    for request, got, want in zip(requests, actual, normalized(expected)):
        assert json.dumps(got) == json.dumps(want), (request, got, want)
    summary = actual[0]["data"]["summary"]
    assert summary["fiat_currency"] == "EUR"
    assert list(summary["definitions"]) == [
        "count", "by_reason", "attention_count", "waiting_count",
        "blocking_count", "scope_count", "group_count",
    ]


REFUSALS = [
    {"action": "exclude", "reason": "r"},
    {"transaction": None, "group_id": None, "action": "exclude", "reason": "r"},
    {"transaction": "a", "group_id": "root:a", "action": "exclude", "reason": "r"},
    {"group_id": 7, "action": "exclude", "reason": "r"},
    {"group_id": "  ", "action": "exclude", "reason": "r"},
    {"group_id": "root:a", "action": "delete", "reason": "r"},
    {"group_id": "root:a", "action": "exclude"},
    {"group_id": "root:a", "action": "exclude", "reason": "r", "reprocess": "yes"},
    {"group_id": "reason:missing_spot_price", "action": "exclude", "reason": "r"},
    {"group_id": "unrooted:basis_provenance_incomplete", "action": "exclude", "reason": "r"},
    {"group_id": "root:b", "action": "exclude", "reason": "r"},
    {"group_id": "root:missing", "action": "exclude", "reason": "r"},
    {"group_id": "root:a", "action": "price_override", "reason": "r"},
    {"group_id": "root:a", "action": "exclude", "reason": "r", "fiat_rate": "1"},
]


@pytest.mark.parametrize("cipher", [False, True])
def test_resolve_by_group_refusals_match_python_without_writes(tmp_path, environment, cipher):
    reference = held_book(tmp_path / "python", cipher)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    before = masked_book(reference, cipher)
    expected = resolve(reference, environment, False, cipher, REFUSALS)
    actual = resolve(native, environment, True, cipher, REFUSALS)
    for args, got, want in zip(REFUSALS, actual, normalized(expected)):
        assert json.dumps(masked(got)) == json.dumps(masked(want)), (args, got, want)
    assert all(row["kind"] == "error" for row in actual)
    assert masked_book(native, cipher) == masked_book(reference, cipher) == before


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("request_args", [
    {"group_id": " root:a ", "action": "price_override", "fiat_rate": "10000", "reason": "Statement price"},
    {"group_id": "root:a", "action": "price_override", "fiat_value": "100", "reason": "r", "reprocess": False},
    {"group_id": "root:t", "action": "exclude", "reason": "Not ours", "reprocess": False},
    {"group_id": "root:t", "action": "exclude", "reason": "Not ours"},
    {"transaction": "a", "action": "price_override", "fiat_rate": "10000", "reason": "r"},
    {"transaction": "i", "action": "exclude", "reason": "r", "reprocess": False},
], ids=["root-reprocess", "root-unmeasured", "own-root-unmeasured", "own-root-reprocess",
        "transaction-reprocess", "downstream-unmeasured"])
def test_resolve_by_group_reports_the_same_effect_and_book(tmp_path, environment, cipher, request_args):
    reference = held_book(tmp_path / "python", cipher)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    expected = resolve(reference, environment, False, cipher, [request_args])
    actual = resolve(native, environment, True, cipher, [request_args])
    (tmp_path / "resolve.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=1))
    assert json.dumps(masked(actual[0])) == json.dumps(masked(normalized(expected)[0]))
    assert masked_book(native, cipher) == masked_book(reference, cipher)
    data = actual[0]["data"]
    if request_args.get("reprocess", True):
        assert data["quarantine_delta"]["status"] == "measured"
    else:
        assert data["dependents_cleared"] is None
        assert data["quarantine_delta"]["status"] == "not_measured"
