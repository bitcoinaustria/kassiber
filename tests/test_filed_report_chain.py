"""Saved exports, real sync publication, inbox persistence and journal rebuild."""
from dataclasses import replace
import json
import sqlite3
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _wallet_sync_hooks, process_journals
from kassiber.core import custody_filed_reports as reports, filed_report_chain as impacts
from kassiber.core import chain_analysis_watches as watches
from kassiber.core.sync import sync_wallet_from_backend
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from kassiber.db import open_db
from kassiber.chain_analysis_watches_schema import ensure_inbox_sources
from tests import test_rp2_ownership_transfers as ownership_fixtures
from tests.test_source_overlap import ADDR_A, _script

TX = "aa" * 32
PROFILE = "profile-1"


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path / "book")
    ownership_fixtures.OwnershipDeriverHandlerTest()._seed(conn)
    conn.execute("UPDATE wallets SET kind='address',config_json=? WHERE id='wallet-a'", (json.dumps({"chain": "bitcoin", "network": "main", "addresses": [ADDR_A]}),))
    conn.commit()
    state = {"confirmed": True, "block_hash": "11" * 32, "retracted": False}
    graph = {"txid": TX, "chain": "bitcoin", "network": "main", "fee": 10,
             "vin": [{"txid": "bb" * 32, "vout": 0, "prevout": {"scriptpubkey": "0014" + "ff" * 20, "value": 100010}}],
             "vout": [{"n": 0, "scriptpubkey": _script(ADDR_A), "value": 100000}]}

    def adapter(_backend, _wallet, sync_state):
        if state["retracted"]:
            return [], {"observer_retracted_external_ids": [TX]}
        status = {"confirmed": state["confirmed"]}
        if state["confirmed"]:
            status.update(block_time=1767225600, block_height=100, block_hash=state["block_hash"])
        return [record_from_bitcoin_esplora_tx({**graph, "status": status}, sync_state.tracked_scripts, "esplora")], {}

    hooks = replace(_wallet_sync_hooks(commit=False), prepare_observer_fetch=None,
        resolve_backend=lambda *_: {"name": "fixture", "kind": "esplora", "url": "https://example.invalid"},
        backend_adapters={"esplora": adapter})

    def sync(*, changed_hooks=None):
        profile = conn.execute("SELECT * FROM profiles WHERE id=?", (PROFILE,)).fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id='wallet-a'").fetchone()
        return sync_wallet_from_backend(conn, {}, profile, wallet, changed_hooks or hooks)

    sync()
    conn.execute("UPDATE transactions SET fiat_rate=40000")
    conn.commit()
    process_journals(conn, "Main", "Default")
    artifact = tmp_path / "report.pdf"
    artifact.write_bytes(b"completed export fixture")

    def export(**kwargs):
        return reports.register_saved_report_export(conn, workspace_id="ws-1", profile_id=PROFILE,
            report_kind="full-report.pdf", artifact_paths=[artifact], period_start_year=2026, period_end_year=2026, **kwargs)

    yield conn, state, sync, export, hooks
    conn.close()


def test_export_retraction_inbox_and_rebuild_preserve_original(book):
    conn, state, sync, export, _ = book
    snapshot = export()
    assert conn.execute("SELECT COUNT(*) FROM filed_report_chain_dependencies").fetchone()[0] == 1
    assert watches.inbox(conn, PROFILE, {})["items"] == []
    assert sync()["saved_report_impacts"] == 0
    state["retracted"] = True
    assert sync()["saved_report_impacts"] == 1
    assert sync()["saved_report_impacts"] == 0
    conn.commit()
    inbox = watches.inbox(conn, PROFILE, {})
    item = inbox["items"][0]
    assert inbox["unread_count"] == 1
    assert item["watch_id"] is None
    assert item["code"] == "report_input_retracted"
    assert item["observation"]["before"]["confirmed"] is True
    assert item["observation"]["after"]["status"] == "retracted"
    assert item["report_impact"]["snapshot_id"] == snapshot["id"]
    assert item["report_impact"]["transaction_available"] is False
    process_journals(conn, "Main", "Default")
    resolved = watches.inbox(conn, PROFILE, {})["items"][0]["report_impact"]
    assert resolved["resolution"]["amendment_status"] == "saved_report_changed"
    assert reports.get_filed_report_snapshot(conn, snapshot["id"]) == snapshot
    watches.acknowledge(conn, PROFILE, {"id": item["id"]})
    assert watches.inbox(conn, PROFILE, {})["unread_count"] == 0
    assert impacts.resolve_pending_chain_impacts(conn, PROFILE, "again") == 0
    with pytest.raises(sqlite3.IntegrityError):
        conn.execute("UPDATE filed_report_chain_resolutions SET rebuilt_at='changed'")


def test_empty_scope_and_unbooked_observation_are_not_dependencies(book):
    conn, state, sync, export, _ = book
    snapshot = export(report_scope={"wallet_ids": ["wallet-b"]})
    assert conn.execute("SELECT COUNT(*) FROM filed_report_chain_dependencies WHERE snapshot_id=?", (snapshot["id"],)).fetchone()[0] == 0
    # A row with current observer provenance but no booked journal entry must
    # never become a report input (e.g. quarantine or excluded evidence).
    original_id = conn.execute("SELECT id FROM transactions").fetchone()[0]
    conn.execute("DELETE FROM journal_entries")
    snapshot2 = export()
    assert conn.execute("SELECT COUNT(*) FROM filed_report_chain_dependencies WHERE snapshot_id=?", (snapshot2["id"],)).fetchone()[0] == 0
    state["retracted"] = True
    assert sync()["saved_report_impacts"] == 0
    assert watches.inbox(conn, PROFILE, {})["items"] == []
    assert original_id


def test_capture_keeps_historical_cross_wallet_basis_but_not_unrelated_inputs(book):
    from tests.custody_tax_helpers import persist_authoritative_chain_observation
    conn, _state, _sync, export, _hooks = book
    original = dict(conn.execute("SELECT * FROM transactions").fetchone())
    entry = dict(conn.execute("SELECT * FROM journal_entries LIMIT 1").fetchone())
    for index, (ident, year, asset, excluded, booked) in enumerate((
        ("prior-basis", 2024, "BTC", 0, True),
        ("future", 2030, "BTC", 0, True),
        ("excluded", 2024, "BTC", 1, True),
        ("other-asset", 2024, "LBTC", 0, True),
        ("unbooked", 2024, "BTC", 0, False),
    ), start=1):
        tx = {**original, "id": ident, "wallet_id": "wallet-b", "external_id": f"{index:064x}",
              "fingerprint": ident, "occurred_at": f"{year}-01-01T00:00:00Z", "asset": asset, "excluded": excluded}
        raw = json.loads(tx["raw_json"])
        raw["txid"] = tx["external_id"]
        tx["raw_json"] = json.dumps(raw)
        keys = list(tx)
        conn.execute(f"INSERT INTO transactions({','.join(keys)}) VALUES({','.join('?' for _ in keys)})", [tx[key] for key in keys])
        persist_authoritative_chain_observation(conn, ident)
        if booked:
            je = {**entry, "id": "entry-" + ident, "transaction_id": ident, "wallet_id": "wallet-b", "occurred_at": tx["occurred_at"], "asset": asset}
            keys = list(je)
            conn.execute(f"INSERT INTO journal_entries({','.join(keys)}) VALUES({','.join('?' for _ in keys)})", [je[key] for key in keys])
    snapshot = export(report_scope={"wallet_ids": ["wallet-a"]})
    dependency_ids = {row[0] for row in conn.execute("SELECT transaction_id FROM filed_report_chain_dependencies WHERE snapshot_id=?", (snapshot["id"],))}
    assert dependency_ids == {original["id"], "prior-basis"}


def test_confirmation_reversal_waits_for_authoritative_recovery(book):
    conn, state, sync, export, _ = book
    snapshot = export()
    state["confirmed"] = False
    assert sync()["saved_report_impacts"] == 1
    item = watches.inbox(conn, PROFILE, {})["items"][0]
    assert item["code"] == "threshold_reversed"
    process_journals(conn, "Main", "Default")
    assert watches.inbox(conn, PROFILE, {})["items"][0]["report_impact"]["resolution"] is None
    state["confirmed"] = True
    state["block_hash"] = "22" * 32
    assert sync()["saved_report_impacts"] == 1
    assert sync()["saved_report_impacts"] == 0
    process_journals(conn, "Main", "Default")
    assert all(item["report_impact"]["resolution"]["amendment_status"] == "no_change" for item in watches.inbox(conn, PROFILE, {})["items"])
    assert reports.get_filed_report_snapshot(conn, snapshot["id"]) == snapshot


def test_missing_authority_is_coverage_loss_not_retraction(book):
    conn, state, sync, export, _ = book
    export()
    conn.execute("DELETE FROM chain_observation_provenance")
    assert impacts.record_wallet_changes(conn, PROFILE, "wallet-a") == 1
    item = watches.inbox(conn, PROFILE, {})["items"][0]
    assert item["code"] == "coverage_lost"
    assert item["observation"]["after"]["status"] == "unavailable"
    process_journals(conn, "Main", "Default")
    assert impacts.resolve_pending_chain_impacts(conn, PROFILE, "unproven") == 0
    assert sync()["saved_report_impacts"] == 1
    process_journals(conn, "Main", "Default")
    assert watches.inbox(conn, PROFILE, {})["items"][-1]["report_impact"]["resolution"] is not None


def test_sync_rollback_rolls_back_report_items_and_authority(book):
    conn, state, sync, export, hooks = book
    export()
    state["confirmed"] = False
    conn.execute("SAVEPOINT wallet_apply")
    def failure(stage):
        if stage == "transaction_insertion":
            raise RuntimeError("later publication failure")
    # The same caller-owned savepoint as production sync must own new inbox
    # writes; fail after insertion and prove no notification survives rollback.
    with patch("kassiber.core.sync.notify_apply_stage", side_effect=lambda _hooks, stage: failure(stage)):
        with pytest.raises(RuntimeError):
            sync()
    conn.execute("ROLLBACK TO wallet_apply")
    conn.execute("RELEASE wallet_apply")
    assert watches.inbox(conn, PROFILE, {})["items"] == []
    assert sync()["saved_report_impacts"] == 1


def test_inbox_paging_scope_read_and_filed_state_never_mutate(book):
    conn, state, sync, export, _ = book
    saved = export()
    # Register the same already-computed result explicitly as filed. Dependency
    # capture is explicit here; arbitrary external reports cannot gain it.
    filed = reports.create_filed_report_snapshot(conn, workspace_id="ws-1", profile_id=PROFILE,
        report_kind=saved["report_kind"], report_state="filed", period_start_year=2026,
        period_end_year=2026, content_sha256=saved["content_sha256"],
        classification_summary=saved["classification_summary"], gain_summary=saved["gain_summary"])
    impacts.capture_export_dependencies(conn, filed)
    state["retracted"] = True
    assert sync()["saved_report_impacts"] == 2
    page = watches.inbox(conn, PROFILE, {"limit": 1})
    assert page["next_cursor"] is not None
    second = watches.inbox(conn, PROFILE, {"limit": 1, "before": page["next_cursor"]})
    assert page["items"][0]["id"] != second["items"][0]["id"]
    assert watches.inbox(conn, "another-profile", {})["items"] == []
    process_journals(conn, "Main", "Default")
    items = watches.inbox(conn, PROFILE, {})["items"]
    filed_item = next(i for i in items if i["report_impact"]["report_state"] == "filed")
    assert filed_item["report_impact"]["resolution"]["amendment_status"] == "review_required"
    watches.acknowledge(conn, PROFILE, {"id": filed_item["id"]})
    assert reports.get_filed_report_snapshot(conn, filed["id"]) == filed


def test_existing_confirmation_watch_marks_report_uncertain_until_wallet_refresh(book):
    conn, state, sync, export, _ = book
    export()
    definition = {"rule": "confirmations", "query": {"subject": f"bitcoin:main:tx:{TX}"}}
    impacts.record_watch_change(conn, PROFILE, definition, {"status": "unavailable"}, "coverage_lost")
    assert watches.inbox(conn, PROFILE, {})["items"][0]["observation"]["after"]["reason"] == "watch_coverage_lost"
    process_journals(conn, "Main", "Default")
    assert impacts.resolve_pending_chain_impacts(conn, PROFILE, "not-proof") == 0
    assert sync()["saved_report_impacts"] == 1
    process_journals(conn, "Main", "Default")
    assert all(i["report_impact"]["resolution"] for i in watches.inbox(conn, PROFILE, {})["items"])


def test_watch_evaluation_appends_report_attachment_in_same_durable_inbox(book):
    conn, _state, _sync, export, _ = book
    export()
    definition = {"rule": "confirmations", "query": {"subject": TX, "chain": "bitcoin", "network": "main"}}
    with patch.object(watches, "require_encrypted"), patch.object(watches, "domain_for", return_value={"chain": "bitcoin", "network": "main"}), patch.object(watches, "revision_for", return_value={"revision": 1}), patch.object(watches, "observe", return_value={"status": "observed", "values": [True]}):
        watch = watches.create(conn, PROFILE, {"plan": watches.preview(conn, PROFILE, definition)})
    with patch.object(watches, "require_encrypted"), patch.object(watches, "domain_for", return_value={"chain": "bitcoin", "network": "main"}), patch.object(watches, "revision_for", return_value={"revision": 2}), patch.object(watches, "observe", return_value={"status": "unavailable", "values": []}):
        assert watches.evaluate_due(conn, PROFILE)["event_count"] == 1
        assert watches.evaluate_due(conn, PROFILE)["event_count"] == 0
    inbox = watches.inbox(conn, PROFILE, {})
    assert inbox["unread_count"] == 2
    assert len([item for item in inbox["items"] if item.get("report_impact")]) == 1
    watches.delete(conn, PROFILE, {"id": watch["id"], "expected_revision": 1})
    assert watches.inbox(conn, PROFILE, {})["items"][0]["report_impact"]


def test_inbox_migration_preserves_sparse_cursors_and_delivery_receipts():
    conn = sqlite3.connect(":memory:")
    conn.execute("CREATE TABLE chain_analysis_watch_inbox(id TEXT PRIMARY KEY,watch_id TEXT NOT NULL,profile_id TEXT,sequence INTEGER,code TEXT,observation_json TEXT,created_at TEXT,acknowledged_at TEXT,delivered_at TEXT)")
    conn.execute("INSERT INTO chain_analysis_watch_inbox VALUES('event','watch','profile',2,'coverage_lost','{}','then','read','delivered')")
    conn.execute("UPDATE chain_analysis_watch_inbox SET rowid=17")
    ensure_inbox_sources(conn)
    row = conn.execute("SELECT rowid,* FROM chain_analysis_watch_inbox").fetchone()
    assert row == (17, 'event', 'watch', 'profile', 2, 'coverage_lost', '{}', 'then', 'read', 'delivered')
    assert next(row for row in conn.execute("PRAGMA table_info(chain_analysis_watch_inbox)") if row[1] == 'watch_id')[3] == 0
    ensure_inbox_sources(conn)
    conn.close()


def test_legacy_inbox_migration_with_complete_new_schema():
    from kassiber.db import SCHEMA
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(SCHEMA.replace("watch_id TEXT REFERENCES chain_analysis_watches", "watch_id TEXT NOT NULL REFERENCES chain_analysis_watches"))
    ensure_inbox_sources(conn)
    assert conn.execute("SELECT 1 FROM sqlite_master WHERE name='trg_report_chain_impacts_delete_inbox'").fetchone()
    assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    conn.close()
