"""Coarse-price acceptance (#708) on the Rust backend against unchanged Python.

A book that requires coarse review holds a coarse price for review unless the
owner accepted that exact price. ``journals process`` must give the same
envelope and leave the same book (journals, quarantines, ledger) on both
sides, and the transaction history kinds must render and revert the
acceptance the same way. The acceptance itself is written by Python's
metadata core before the book is copied, as a review apply writes it.
"""
import json
import shutil

import pytest

from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import metadata
from kassiber.db import open_db
from tests.test_rust_journal_commands import BINARY, KEY, book_dump, invoke, seed
from tests.test_rust_journal_commands import environment  # noqa: F401
from tests.test_rust_journal_process import scalar_exact
from tests.test_rust_journal_quarantine_resolve import book as masked_book, masked
from tests.test_rust_journal_views import Reads, normalized

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")

COARSE = ("b", "s")
# shape -> transactions held with pricing_review_required after processing
SHAPES = {
    "unaccepted": {"b", "s"},
    "accepted": set(),
    "accepted-trailing-zero": set(),
    "stale": {"b"},
    "value-accepted": set(),
    "value-then-rate": {"b"},
    "not-required": set(),
}


def coarse_book(root, cipher, shape):
    seed(root, cipher=cipher)
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        conn.execute("UPDATE profiles SET require_coarse_review=?", (int(shape != "not-required"),))
        conn.execute(
            "UPDATE transactions SET pricing_quality='coarse_fallback',"
            "pricing_source_kind='fmv_provider',pricing_granularity='daily' "
            f"WHERE id IN {COARSE}"
        )
        if shape.startswith("value-"):
            conn.execute(
                "UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,"
                "fiat_value=20000.0,fiat_value_exact='20000.0' WHERE id='b'"
            )
        conn.commit()
        if shape not in ("unaccepted", "not-required"):
            for tx in COARSE:
                metadata.update_transaction_metadata(
                    conn, None, None, tx, _metadata_hooks(),
                    accept_coarse_price=True, source="gui", reason="Daily averages reviewed",
                )
            # Before both runtimes' revert times: Python's reference edits at
            # the fixed test clock, Rust's at the system clock.
            conn.execute("UPDATE transaction_edit_events SET changed_at='2020-06-01T00:00:00Z'")
        if shape == "accepted-trailing-zero":
            # A rebuilt rate with trailing zeros is the same accepted price.
            conn.execute("UPDATE transactions SET fiat_rate_exact='20000.00' WHERE id='b'")
        elif shape == "stale":
            conn.execute("UPDATE transactions SET fiat_rate=20001.0,fiat_rate_exact='20001' WHERE id='b'")
        elif shape == "value-then-rate":
            # The acceptance names the value; a rate now prices the row.
            conn.execute("UPDATE transactions SET fiat_rate=20000.0,fiat_rate_exact='20000' WHERE id='b'")
        conn.commit()
    finally:
        conn.close()
    return root


def without_recount(dump):
    """Python's rebuild recounts transfer candidates since #707 (main-sync
    slice S4, not ported yet): one cached count on the profile, whose write
    bumps the chain-index clock. Mask exactly those two effects."""
    profiles = dump["profiles"]
    index = profiles["columns"].index("swap_candidate_count")
    profiles["rows"] = [[*row[:index], None, *row[index + 1:]] for row in profiles["rows"]]
    for name in ("chain_index_clock", "chain_index_dirty"):
        table = dump[name]
        index = table["columns"].index("revision")
        table["rows"] = [[*row[:index], None, *row[index + 1:]] for row in table["rows"]]
    return dump


def held(root, cipher):
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        return {
            row[0] for row in conn.execute(
                "SELECT transaction_id FROM journal_quarantines WHERE reason='pricing_review_required'"
            )
        }
    finally:
        conn.close()


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("shape", sorted(SHAPES))
def test_journals_process_holds_only_unaccepted_or_changed_coarse_prices(tmp_path, environment, cipher, shape):
    reference = coarse_book(tmp_path / "python", cipher, shape)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    expected = invoke(reference, environment, cipher=cipher)
    actual = invoke(native, environment, cipher=cipher, native=True)
    assert expected.returncode == actual.returncode == 0, (expected, actual)
    assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    assert scalar_exact(without_recount(book_dump(native, cipher)), without_recount(book_dump(reference, cipher)))
    assert held(native, cipher) == SHAPES[shape]


def acceptance_events(root, cipher):
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        return [
            (row[0], row[1]) for row in conn.execute(
                "SELECT e.transaction_id, e.id FROM transaction_edit_events e "
                "JOIN transaction_edit_fields f ON f.event_id=e.id "
                "WHERE f.field='pricing_review_accepted' ORDER BY e.transaction_id"
            )
        ]
    finally:
        conn.close()


def history_requests(events):
    reads = [
        ("ui.transactions.history", {"transaction": "b"}),
        ("ui.transactions.history", {"transaction": "b", "field": "pricing_review_accepted"}),
        ("ui.transactions.history", {"transaction": "s", "field_family": "pricing"}),
        ("ui.transactions.history", {"transaction": "b", "field": "bogus"}),
        ("ui.activity.history", {"field": "pricing_review_accepted"}),
        ("ui.activity.history", {"pricing_only": True}),
    ]
    (b, b_event), (s, s_event) = events
    return [
        *reads,
        ("ui.transactions.history.revert", {"transaction": b, "event": b_event, "reason": "Undo the acceptance"}),
        ("ui.transactions.history.revert",
         {"transaction": s, "event": s_event, "field": "pricing_review_accepted"}),
        # Book-wide reads would order the two reverts by their generated ids
        # within one second; each transaction holds one revert.
        *[(kind, args) for kind, args in reads if kind == "ui.transactions.history"],
    ]


def session(root, env, native, cipher, requests):
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
@pytest.mark.parametrize("shape", ["accepted", "value-accepted"])
def test_history_labels_and_reverts_the_acceptance_like_python(tmp_path, environment, cipher, shape):
    reference = coarse_book(tmp_path / "python", cipher, shape)
    native = tmp_path / "rust"
    shutil.copytree(reference, native)
    events = acceptance_events(reference, cipher)
    assert [tx for tx, _ in events] == list(COARSE)
    requests = history_requests(events)
    expected = session(reference, environment, False, cipher, requests)
    actual = session(native, environment, True, cipher, requests)
    (tmp_path / "history.json").write_text(json.dumps({"actual": actual, "expected": expected}, indent=1))
    for request, got, want in zip(requests, actual, normalized(expected)):
        assert json.dumps(masked(got)) == json.dumps(masked(want)), (request, got, want)
    labels = [
        field["after_label"]
        for event in actual[1]["data"]["events"]
        for field in event["fields"]
    ]
    assert labels == [
        "Accepted value 20000.0" if shape == "value-accepted" else "Accepted price per unit 20000"
    ]
    reverted = [row for (kind, _), row in zip(requests, actual) if kind.endswith(".revert")]
    assert all(row["data"]["updated"] for row in reverted), reverted
    assert masked_book(native, cipher) == masked_book(reference, cipher)
    conn = open_db(native, passphrase=KEY if cipher else None)
    try:
        assert conn.execute(
            "SELECT COUNT(*) FROM transactions WHERE pricing_review_accepted_price IS NOT NULL"
        ).fetchone()[0] == 0
    finally:
        conn.close()
