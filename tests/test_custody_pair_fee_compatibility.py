"""Old pair authoring must never book its observed fee twice.

The fixture was written by c0a015de219348e7ed427abfe777769b91bfd374
using create_transaction_pair, as in the reviewer's scripts/stale_pair.py.
"""

import base64
import gzip
import json
import sqlite3
from contextlib import closing
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


# Later main migrations invalidate journals once before the pair fee review
# (kassiber-tax-engine-v1) and add transactions columns, so compare only the
# columns the fixture was written with.
FIXTURE_INPUT_VERSION = 7
PAIR_FEE_INPUT_VERSION = 9
AUTHORED = ("transactions", "custody_components", "custody_component_legs",
            "custody_component_allocations", "custody_component_economic_terms")


def _columns(conn):
    return {table: [row[1] for row in conn.execute(f"PRAGMA table_info({table})")] for table in AUTHORED}


def _authored(conn, columns=None):
    columns = columns or _columns(conn)
    return {
        table: [tuple(row) for row in conn.execute(
            f"SELECT {', '.join(columns[table])} FROM {table} ORDER BY id")]
        for table in AUTHORED
    }


def test_base_code_pair_is_blocked_and_audited_once_without_rewriting_economics(tmp_path):
    root = tmp_path / "book"
    path = _restore(root)
    with closing(sqlite3.connect(path)) as original, original:
        columns = _columns(original)
        before = _authored(original, columns)
    for _ in range(2):
        conn = open_db(root)
        try:
            component = get_component(conn, before["custody_components"][0][0])
            assert component["state"] == "active"
            assert component["effective_state"] == "draft"
            assert ISSUE in {item["code"] for item in component["validation"]["issues"]}
            assert _authored(conn, columns) == before
            profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
            assert profile["journal_input_version"] == PAIR_FEE_INPUT_VERSION
            assert profile["last_processed_input_version"] == FIXTURE_INPUT_VERSION
            assert profile["last_processed_at"] is None
            assert profile["required_journal_semantics_version"] == 7
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
        assert profile["journal_input_version"] == PAIR_FEE_INPUT_VERSION
        assert profile["required_journal_semantics_version"] == 7
        assert conn.execute("SELECT COUNT(*) FROM schema_migration_audits WHERE migration_name=?", (MIGRATION,)).fetchone()[0] == 1
    finally:
        conn.close()


def test_pair_fee_audit_and_invalidation_roll_back_together_on_interruption(tmp_path):
    root = tmp_path / "book"
    path = _restore(root)
    with closing(sqlite3.connect(path)) as conn, conn:
        # Interrupt the pair fee review's own invalidation, after main's.
        conn.execute("""CREATE TRIGGER stop_pair_fee_migration BEFORE UPDATE ON profiles
            WHEN NEW.journal_input_version != OLD.journal_input_version
            AND OLD.journal_input_version = ?
            BEGIN SELECT RAISE(ABORT, 'injected migration interruption'); END"""
            .replace("?", str(PAIR_FEE_INPUT_VERSION - 1)))
    with pytest.raises(Exception, match="injected migration interruption"):
        open_db(root)
    with closing(sqlite3.connect(path)) as conn, conn:
        assert conn.execute("SELECT 1 FROM schema_migration_audits WHERE migration_name=?", (MIGRATION,)).fetchone() is None
        assert conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0] == PAIR_FEE_INPUT_VERSION - 1
        conn.execute("DROP TRIGGER stop_pair_fee_migration")
    conn = open_db(root)
    try:
        assert conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0] == PAIR_FEE_INPUT_VERSION
    finally:
        conn.close()


def test_legacy_pair_lists_the_specific_review_code_and_concrete_repair(tmp_path):
    from kassiber.core import custody_review_terms, custody_journal, quarantine_review, review_workflow
    root = tmp_path / "book"
    _restore(root)
    conn = open_db(root)
    try:
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        pair, = custody_review_terms.list_pair_reviews(conn, profile["id"])
        assert ISSUE in pair["review_codes"]
        assert pair["review_required"]
        assert f"transfers update --pair-id {pair['id']} --note" in pair["repair"]["command"]
        state = review_workflow._build(conn, profile)
        custody_journal.store_ledger_state(conn, profile, state)
        conn.commit()
        items = quarantine_review.review_quarantine(conn, profile, limit=20)["items"]
        assert items
        for item in items:
            assert ISSUE in item["evidence"]["review_codes"]
            assert item["evidence"]["repair"] == pair["repair"]
            assert any(action["kind"] == "review_pair" and action["pair_id"] == pair["id"]
                       and action["transaction_id"] == item["transaction_id"]
                       for action in item["actions"])
    finally:
        conn.close()


def test_legacy_pair_repair_is_read_only_in_preview_then_explicit_and_version_bound(tmp_path):
    from kassiber.cli.handlers import _metadata_hooks
    from kassiber.core import custody_review_terms, review_workflow as review
    root = tmp_path / "book"
    _restore(root)
    conn = open_db(root)
    try:
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        before = _authored(conn)
        cases = review.inspect_cases(conn, profile)["cases"]
        case = next(case for case in cases if "repair_legacy_pair" in case["supported_operations"])
        operation = {"type": "repair_legacy_pair", "pair_id": case["pair"]["pair_id"],
                     "expected_fingerprint": case["pair"]["pair_fingerprint"],
                     "reason": "Reviewed complete debit and principal residual"}
        from kassiber.daemon import _validate_ai_review_operations
        with pytest.raises(AppError) as ai_error:
            _validate_ai_review_operations([operation])
        assert ai_error.value.code == "interaction_required"
        hooks = review.ReviewHooks(metadata=_metadata_hooks())
        plan = review.plan_review(conn, profile, operations=[operation],
                                  expected_input_version=profile["journal_input_version"], hooks=hooks)
        assert _authored(conn) == before
        assert all(item["reason"] != "custody_component_authored_active_invalid"
                   for item in plan["after"]["quarantines"])
        # The preview names the revised amounts, not only a journal digest.
        def amounts(side):
            terms, = plan[side]["legacy_pairs"]
            assert terms["pair_id"] == operation["pair_id"]
            return ({leg["role"]: leg["amount_msat"] for leg in terms["legs"]},
                    {(row["source"], row["sink"]): row["source_amount_msat"] for row in terms["allocations"]})
        assert amounts("before") == ({"source": 1000, "destination": 900, "fee": 10, "suspense": 90},
                                     {("source", "destination"): 900, ("source", "fee"): 10, ("source", "suspense"): 90})
        assert amounts("after") == ({"source": 1010, "destination": 900, "fee": 10, "suspense": 100},
                                    {("source", "destination"): 900, ("source", "fee"): 10, ("source", "suspense"): 100})
        stale = {**operation, "expected_fingerprint": "stale"}
        with pytest.raises(AppError) as raised:
            review.plan_review(conn, profile, operations=[stale],
                               expected_input_version=profile["journal_input_version"], hooks=hooks)
        assert raised.value.code == "review_case_changed"
        receipt = review.apply_review(conn, profile, artifact=plan, idempotency_key="pair-repair", hooks=hooks)
        assert receipt["status"] == "verified"
        pair, = custody_review_terms.list_pair_reviews(conn, profile["id"])
        component = get_component(conn, pair["component_id"])
        assert component["effective_state"] == "active"
        assert not pair["review_required"]
        assert len(_authored(conn)["custody_components"]) == len(before["custody_components"]) + 1
        assert _authored(conn)["transactions"] == before["transactions"]
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
