"""An unchanged wallet row can still supply a newer confirmation observation."""

from dataclasses import replace
import json
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _wallet_sync_hooks, process_journals
from kassiber.core.custody_journal import CustodyJournalBuilder, projection_freshness
from kassiber.core.sync import sync_wallet_from_backend
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from kassiber.db import open_db
from tests import test_rp2_ownership_transfers as fixtures
from tests.test_source_overlap import ADDR_A, ADDR_B, _script


@pytest.mark.parametrize("scenario", ["chronology", "authority"])
def test_duplicate_sync_invalidates_only_changed_observation_inputs(tmp_path, scenario):
    conn = open_db(tmp_path)
    try:
        fixtures.OwnershipDeriverHandlerTest()._seed(conn)
        for wallet_id, address in (("wallet-a", ADDR_A), ("wallet-b", ADDR_B)):
            conn.execute(
                "UPDATE wallets SET kind='address', config_json=? WHERE id=?",
                (json.dumps({"chain": "bitcoin", "network": "main", "addresses": [address]}), wallet_id),
            )
        conn.commit()
        graph = {
            "txid": "aa" * 32, "chain": "bitcoin", "network": "main", "fee": 10,
            "observed_at": 1767225600,
            "vin": [{"txid": "bb" * 32, "vout": 0,
                     "prevout": {"scriptpubkey": "0014" + "ff" * 20, "value": 300010}}],
            "vout": [{"n": 0, "scriptpubkey": _script(ADDR_A), "value": 100000},
                     {"n": 1, "scriptpubkey": _script(ADDR_B), "value": 200000}],
        }

        def adapter(_backend, wallet, state):
            status = {"confirmed": scenario == "authority" or wallet["id"] == "wallet-b"}
            if status["confirmed"]:
                status.update(block_time=1767225600, block_height=100)
            observed = {**graph, "status": status}
            return [record_from_bitcoin_esplora_tx(observed, state.tracked_scripts, "esplora")], {}

        hooks = replace(
            _wallet_sync_hooks(), prepare_observer_fetch=None,
            resolve_backend=lambda *_: {"name": "fixture", "kind": "esplora", "url": "https://example.invalid"},
            backend_adapters={"esplora": adapter},
        )
        profile = conn.execute("SELECT * FROM profiles WHERE id='profile-1'").fetchone()
        wallets = {row["id"]: row for row in conn.execute("SELECT * FROM wallets")}

        def sync(wallet_id, day):
            with patch("kassiber.core.chain_observer.provenance.now_iso", return_value=f"2030-01-{day:02}T00:00:00Z"):
                return sync_wallet_from_backend(conn, {}, profile, wallets[wallet_id], hooks)

        sync("wallet-a", 1)
        sync("wallet-b", 2)
        conn.execute("UPDATE transactions SET fiat_rate=40000")
        conn.commit()
        process_journals(conn, "Main", "Default")
        assert projection_freshness(conn, "profile-1")["is_current"]
        assert conn.execute("SELECT COUNT(*) FROM journal_entries").fetchone()[0] == 2

        if scenario == "authority":
            for day, mutation in enumerate((
                "DELETE FROM chain_observation_provenance WHERE wallet_id='wallet-a'",
                "UPDATE chain_observation_provenance SET quantity_hash='stale' WHERE wallet_id='wallet-a'",
                "UPDATE chain_observation_provenance SET observer_kinds_json='[\"other\"]' WHERE wallet_id='wallet-a'",
            ), start=3):
                conn.execute(mutation)
                process_journals(conn, "Main", "Default")
                assert conn.execute("SELECT COUNT(*) FROM journal_entries").fetchone()[0] == 2
                outcome = sync("wallet-a", day)
                assert outcome["imported"] == outcome["updated"] == 0
                assert outcome["journal_invalidated"]
                assert not projection_freshness(conn, "profile-1")["is_current"]
                process_journals(conn, "Main", "Default")
                steady = sync("wallet-a", day)
                assert not steady["journal_invalidated"]
                assert projection_freshness(conn, "profile-1")["is_current"]
            return

        # The imported row does not change; only its closed observation becomes
        # newer than the other wallet's contradictory confirmation.
        for wallet_id, day, changes, expected_entries in (
            ("wallet-a", 3, True, 0),
            ("wallet-a", 4, False, 0),
            ("wallet-b", 5, True, 2),
            ("wallet-b", 6, False, 2),
        ):
            before = projection_freshness(conn, "profile-1")
            outcome = sync(wallet_id, day)
            assert outcome["imported"] == 0
            assert outcome["updated"] == 0
            assert outcome["journal_invalidated"] is changes
            after = projection_freshness(conn, "profile-1")
            assert after["is_current"] is not changes
            assert after["journal_input_version"] == before["journal_input_version"] + int(changes)
            fresh_profile = conn.execute("SELECT * FROM profiles WHERE id='profile-1'").fetchone()
            assert len(CustodyJournalBuilder(conn, fresh_profile).build()["entries"]) == expected_entries
            if changes:
                process_journals(conn, "Main", "Default")
            assert conn.execute("SELECT COUNT(*) FROM journal_entries").fetchone()[0] == expected_entries
    finally:
        conn.close()
