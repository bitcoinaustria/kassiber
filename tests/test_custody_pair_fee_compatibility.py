"""Old pair authoring must never book its observed fee twice.

The fixture was written by c0a015de219348e7ed427abfe777769b91bfd374
using create_transaction_pair, as in the reviewer's scripts/stale_pair.py.
"""

import base64
import gzip
import json
import sqlite3
from pathlib import Path

import pytest

from kassiber.core.custody_components import get_component
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.db import open_db
from kassiber.errors import AppError


MIGRATION = "custody-legacy-pair-fee-review-v1"
ISSUE = "legacy_pair_fee_allocation_review_required"


def _restore(root):
    root.mkdir()
    fixture = Path(__file__).parent / "fixtures/historical/pre_pair_fee_c0a015de.sqlite3.gz.b64"
    path = root / "kassiber.sqlite3"
    path.write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
    return path


def _authored(conn):
    return {
        table: [tuple(row) for row in conn.execute(f"SELECT * FROM {table} ORDER BY id")]
        for table in ("transactions", "custody_components", "custody_component_legs",
                      "custody_component_allocations", "custody_component_economic_terms")
    }


def test_base_code_pair_is_blocked_and_audited_once_without_rewriting_economics(tmp_path):
    root = tmp_path / "book"
    path = _restore(root)
    with sqlite3.connect(path) as original:
        before = _authored(original)
    for _ in range(2):
        conn = open_db(root)
        try:
            component = get_component(conn, before["custody_components"][0][0])
            assert component["state"] == "active"
            assert component["effective_state"] == "draft"
            assert ISSUE in {item["code"] for item in component["validation"]["issues"]}
            assert _authored(conn) == before
            profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
            assert profile["journal_input_version"] == 8
            assert profile["last_processed_input_version"] == 7
            assert profile["last_processed_at"] is None
            assert profile["required_journal_semantics_version"] == 3
            audits = conn.execute("SELECT impact_json FROM schema_migration_audits WHERE migration_name=?", (MIGRATION,)).fetchall()
            assert len(audits) == 1
            impact = json.loads(audits[0][0])
            assert impact["affected_profile_count"] == 1
            assert impact["affected_component_count"] == 1
            assert component["id"] not in audits[0][0]
            decisions = CustodyJournalBuilder(conn, profile).build_custody_decisions()
            assert any(ISSUE in blocker["issue_codes"] for blocker in decisions.component_blockers)
            assert any(issue.reason == "custody_component_authored_active_invalid"
                       and set(issue.transaction_ids) == {"out", "in"}
                       for issue in decisions.quantity_state.issues)
            assert not decisions.custody_transfers
        finally:
            conn.close()


def test_older_binary_rebuild_cannot_clear_pair_fee_review_requirement(tmp_path):
    from kassiber.core.custody_journal import projection_freshness

    root = tmp_path / "book"
    _restore(root)
    conn = open_db(root)
    try:
        # Version-one writers stamp their semantics after updating the markers.
        conn.execute("""UPDATE profiles SET last_processed_at = ?,
            last_processed_tx_count = 2,
            last_processed_input_version = journal_input_version
            WHERE id = 'profile'""", ("2026-01-01T00:00:00Z",))
        conn.execute("UPDATE profiles SET journal_semantics_version = 1 WHERE id = 'profile'")
        conn.commit()
    finally:
        conn.close()
    conn = open_db(root)
    try:
        assert not projection_freshness(conn, "profile")["is_current"]
        profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
        assert profile["journal_input_version"] == 8
        assert profile["required_journal_semantics_version"] == 3
        assert conn.execute("SELECT COUNT(*) FROM schema_migration_audits WHERE migration_name=?", (MIGRATION,)).fetchone()[0] == 1
    finally:
        conn.close()


def test_pair_fee_audit_and_invalidation_roll_back_together_on_interruption(tmp_path):
    root = tmp_path / "book"
    path = _restore(root)
    with sqlite3.connect(path) as conn:
        conn.execute("""CREATE TRIGGER stop_pair_fee_migration BEFORE UPDATE ON profiles
            WHEN NEW.journal_input_version != OLD.journal_input_version
            BEGIN SELECT RAISE(ABORT, 'injected migration interruption'); END""")
    with pytest.raises(Exception, match="injected migration interruption"):
        open_db(root)
    with sqlite3.connect(path) as conn:
        assert conn.execute("SELECT 1 FROM schema_migration_audits WHERE migration_name=?", (MIGRATION,)).fetchone() is None
        assert conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0] == 7
        conn.execute("DROP TRIGGER stop_pair_fee_migration")
    conn = open_db(root)
    try:
        assert conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0] == 8
    finally:
        conn.close()


@pytest.mark.parametrize("sources,receipts,refund,expected", [
    ([(1000, 10)], [600, 390], False, (1010, 990, 10, 10)),
    ([(500, 5), (500, 7)], [995], False, (1012, 995, 12, 5)),
    ([(1000, 0)], [990], False, (1000, 990, 0, 10)),
    ([(1000, 10)], [990], False, (1010, 990, 10, 10)),
    ([(1000, 10)], [600, 390], True, None),
])
def test_reviewer_pair_shapes_keep_observed_fees_separate(tmp_path, sources, receipts, refund, expected):
    from kassiber.cli.handlers import create_transaction_pair
    from tests.test_custody_authored_migration import _scope, _tx, NOW

    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        for i, (amount, fee) in enumerate(sources):
            _tx(conn, f"out{i}", "btc", "outbound", "BTC", amount, NOW, fee=fee)
        for i, amount in enumerate(receipts):
            _tx(conn, f"in{i}", "btc" if refund else "btc2", "inbound", "BTC", amount, NOW)
        conn.commit()
        for out in range(len(sources)):
            for incoming in range(len(receipts)):
                if refund and incoming:
                    before = _authored(conn)
                    with pytest.raises(AppError) as raised:
                        create_transaction_pair(conn, "ws", "profile", f"out{out}", f"in{incoming}",
                                                kind="swap-refund", policy="carrying-value")
                    assert raised.value.code == "conflict"
                    assert _authored(conn) == before
                    return
                pair = create_transaction_pair(conn, "ws", "profile", f"out{out}", f"in{incoming}",
                                               **({"kind": "swap-refund", "policy": "carrying-value"} if refund else {}))
        conn.commit()
        component = get_component(conn, pair["component_id"])
        assert component["effective_state"] == "active"
        assert tuple(sum(leg["amount_msat"] for leg in component["legs"] if leg["role"] == role)
                     for role in ("source", "destination", "fee", "suspense")) == expected
    finally:
        conn.close()
