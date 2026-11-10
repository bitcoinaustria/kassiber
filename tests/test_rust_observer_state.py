"""Pure state/reuse parity, not product sync or normalized post-sync dumps."""
from dataclasses import asdict
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from kassiber.core.chain_observer.bdk_persistence import deserialize_changeset, serialize_changeset
from kassiber.core.chain_observer.identity import identities_for_wallet
from kassiber.core.chain_observer.provenance import persist_chain_observation_provenance
from kassiber.core.chain_observer.store import CoveragePoint, load_observer_state, persist_observer_state
from kassiber.core.imports import ImportCoordinatorHooks, insert_wallet_records
from kassiber.core.repo import invalidate_journals
from kassiber.core.sync_backends import _current_observed_txids
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_bdk_observer import _descriptor_wallet
from tests.rust_sync_env import child_env as _env
from tests.rust_fixture_binary import fixture_binary

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires observer_state_fixture build")
VECTORS = ROOT / "crates/wallets/tests/vectors/python_observer_state.json"


def _rust(tmp_path, operation, **payload):
    binary = fixture_binary("observer_state_fixture.exe" if os.name == "nt" else "observer_state_fixture")
    result = subprocess.run([str(binary)], input=json.dumps({"operation": operation, **payload}).encode(),
                            capture_output=True, timeout=15, check=True, env=_env(tmp_path))
    assert result.stderr == b""
    return json.loads(result.stdout)


@pytest.mark.parametrize("index", range(len(json.loads(VECTORS.read_text())["changesets"])))
def test_bdk_all_aggregate_components_match_live_python_binding(tmp_path, index):
    case = json.loads(VECTORS.read_text())["changesets"][index]
    expected = serialize_changeset(deserialize_changeset(case["input"]))
    assert _rust(tmp_path, "changeset", payload=case["input"]) == {"result": expected}
    # Restart/encoding roundtrip has exactly the same aggregate JSON.
    assert _rust(tmp_path, "changeset", payload=expected) == {"result": expected}


@pytest.mark.parametrize("case", json.loads(VECTORS.read_text())["rejected_changesets"], ids=lambda case: case["name"])
def test_bdk_invalid_hex_and_wrong_network_keys_refuse_like_python(tmp_path, case):
    with pytest.raises(Exception):
        deserialize_changeset(case["input"])
    assert _rust(tmp_path, "changeset", payload=case["input"])["error"]["code"] == "observer_state_rebuild_required"


def test_observer_vectors_have_not_drifted_from_python(tmp_path):
    result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts/generate_observer_vectors.py"), "--check"],
                            capture_output=True, timeout=30, env=_env(tmp_path))
    assert result.returncode == 0, result.stderr.decode()


@pytest.fixture
def book(tmp_path):
    root = tmp_path / "book"
    conn = open_db(root)
    wallet, _ = _descriptor_wallet()
    timestamp = "2026-01-01T00:00:00Z"
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES(?, 'Workspace', ?)", (wallet["workspace_id"], timestamp))
    conn.execute("""INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,tax_long_term_days,gains_algorithm,created_at)
                    VALUES(?,?,'Profile','EUR','generic',365,'FIFO',?)""", (wallet["profile_id"], wallet["workspace_id"], timestamp))
    conn.execute("""INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
                    VALUES(?,?,?,'Cold','descriptor',?,?)""", (wallet["id"], wallet["workspace_id"], wallet["profile_id"], wallet["config_json"], timestamp))
    identity = identities_for_wallet(wallet, observer_kind="bdk")[0]
    payload = {"schema_version": 1, "bdk_changeset": json.loads(VECTORS.read_text())["changesets"][1]["expected"],
               "canonical_txids": ["ab" * 32], "record_fingerprint_version": "1:fixture", "record_fingerprints": {"ab" * 32: "f" * 32}}
    coverage = [CoveragePoint("receive", 25, 4, {"branch_index": 0, "observer": "bdk"}),
                CoveragePoint("change", 20, None, {"branch_index": 1, "observer": "bdk"})]
    persist_observer_state(conn, identity, payload, coverage, expected_epoch=None)
    conn.commit()
    database = conn.execute("PRAGMA database_list").fetchone()[2]
    try:
        yield conn, database, identity, wallet
    finally:
        conn.close()


@pytest.mark.parametrize("offline", [False, True])
def test_state_restart_reads_match_python_preserve_complete_book_without_consent(tmp_path, book, monkeypatch, offline):
    conn, database, identity, _ = book
    if offline:
        monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    before = list(conn.iterdump())
    prior = load_observer_state(conn, identity)
    expected = {"prior": prior.payload, "coverage": [asdict(point) for point in prior.coverage],
                "expected_epoch": prior.epoch, "autocommit": True}
    for _ in range(2):
        assert _rust(tmp_path, "load", database=database, identity=asdict(identity)) == {"result": expected}
        assert list(conn.iterdump()) == before


@pytest.mark.parametrize("corruption", ["state-version", "coverage-version", "identity", "state-json", "coverage-json"])
def test_force_rebuild_keeps_epoch_and_old_book_ordinary_refresh_refuses(tmp_path, book, corruption):
    conn, database, identity, _ = book
    statement = {
        "state-version": "UPDATE chain_observer_instances SET state_version=2",
        "coverage-version": "UPDATE chain_observer_coverage SET coverage_version=1",
        "identity": "UPDATE chain_observer_instances SET observer_kind='compatibility'",
        "state-json": "UPDATE chain_observer_instances SET state_json='broken'",
        "coverage-json": "UPDATE chain_observer_coverage SET details_json='[]'",
    }[corruption]
    conn.execute(statement)
    conn.commit()
    before = list(conn.iterdump())
    with pytest.raises(AppError) as error:
        load_observer_state(conn, identity)
    result = _rust(tmp_path, "load", database=database, identity=asdict(identity))
    assert result["error"]["code"] == error.value.code == "observer_state_rebuild_required"
    assert _rust(tmp_path, "load", database=database, identity=asdict(identity), force_full=True) == {
        "result": {"prior": None, "coverage": [], "expected_epoch": 1, "autocommit": True}}
    assert list(conn.iterdump()) == before


def test_current_rows_match_python_after_restart_edits_shared_sources_and_exclusions(tmp_path, book):
    conn, database, identity, wallet = book
    profile = conn.execute("SELECT * FROM profiles WHERE id=?", (wallet["profile_id"],)).fetchone()
    stored_wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (wallet["id"],)).fetchone()
    txids = [f"{n:02x}" * 32 for n in range(1, 7)]
    for txid in txids:
        record = {"txid": txid, "occurred_at": "2026-01-01T00:00:00Z", "confirmed_at": "2026-01-01T00:00:00Z",
                  "direction": "inbound", "asset": "BTC", "amount": "0.001", "fee": "0", "kind": "deposit",
                  "raw_json": json.dumps({"txid": txid, "observer": "bdk", "vin": [], "vout": []})}
        outcome = insert_wallet_records(conn, profile, stored_wallet, [record], "backend:fixture",
                                        ImportCoordinatorHooks(ensure_tag_row=lambda *_: None, invalidate_journals=invalidate_journals),
                                        commit=False, authoritative_chain_observer=True)
        persist_chain_observation_provenance(conn, profile, stored_wallet, application_revision="fixture", chain="bitcoin", network="main",
            entries=[{"external_id": txid, "asset": "BTC", "direction": "inbound", "observer_ids": [identity.id], "observer_kinds": ["bdk"]}],
            resolved_records=outcome["_observer_resolved_records"])
    conn.execute("UPDATE transactions SET amount=amount+1 WHERE external_id=?", (txids[1],))
    conn.execute("UPDATE transactions SET raw_json='{}' WHERE external_id=?", (txids[2],))
    conn.execute("UPDATE transactions SET excluded=1 WHERE external_id=?", (txids[3],))
    conn.execute("UPDATE chain_observation_provenance SET observer_ids_json=? WHERE transaction_id IN (SELECT id FROM transactions WHERE external_id=?)",
                 (json.dumps([identity.id, "another"]), txids[4]))
    conn.execute("DELETE FROM chain_observation_provenance WHERE transaction_id IN (SELECT id FROM transactions WHERE external_id=?)", (txids[5],))
    conn.commit()
    before = list(conn.iterdump())
    expected = sorted(_current_observed_txids(conn, wallet["id"], identity.id))
    assert expected == [txids[0]]
    assert _rust(tmp_path, "current", database=database, wallet_id=wallet["id"], observer_id=identity.id) == {"result": expected}
    assert list(conn.iterdump()) == before
