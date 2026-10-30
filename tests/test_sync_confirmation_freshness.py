"""An unchanged wallet row can still supply a newer confirmation observation."""

from dataclasses import replace
from contextlib import nullcontext
import json
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _wallet_sync_hooks, process_journals
from kassiber.core.custody_journal import CustodyJournalBuilder, projection_freshness
from kassiber.core.sync import sync_wallet_from_backend
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from kassiber.core import sync_backends
from kassiber.core.sync import WalletSyncState
from kassiber.db import open_db
from tests import test_rp2_ownership_transfers as fixtures
from tests.test_source_overlap import ADDR_A, ADDR_B, _script


@pytest.mark.parametrize("kind", ["esplora", "electrum"])
def test_configured_backend_target_resolution_does_not_probe_chain_tips(kind):
    backend = {"name": "fixture", "kind": kind, "url": "https://unused.invalid",
               "chain": "bitcoin", "network": "main"}
    wallet = {"id": "wallet", "kind": "address", "config_json": json.dumps({
        "chain": "bitcoin", "network": "main", "backend": "fixture", "addresses": [ADDR_A],
    })}
    with patch.object(sync_backends, "http_get_text") as http, patch.object(
        sync_backends, "_electrum_client_context",
    ) as electrum:
        assert sync_backends.resolve_wallet_sync_targets(backend, wallet).targets
    http.assert_not_called()
    electrum.assert_not_called()


@pytest.mark.parametrize("tip_change", ["reorg", "advance", "missing", "invalid", "legacy"])
def test_electrum_refreshes_cached_confirmation_when_chain_anchor_changes(tip_change):
    script = _script(ADDR_A)
    target = {"script_pubkey": script}
    txid = "aa" * 32
    timestamp = 1767225600
    tip = {"height": 100, "hex": "00" * 80}
    history = [{"tx_hash": txid, "height": 100}]
    calls = []

    class Client:
        def call(self, method, params):
            calls.append((method, params))
            if method == "blockchain.headers.subscribe":
                return tip
            if method == "blockchain.scripthash.subscribe":
                return "unchanged-status"
            if method == "blockchain.scripthash.get_history":
                return history
            if method == "blockchain.transaction.get":
                return "synthetic-transaction"
            assert method == "blockchain.block.header"
            return (b"\x00" * 68 + timestamp.to_bytes(4, "little") + b"\x00" * 8).hex()

        def batch_call(self, requests):
            return [self.call(method, params) for method, params in requests]

    state = WalletSyncState(chain="bitcoin", network="regtest", descriptor_plan=None,
        policy_asset_id="", targets=[target], tracked_scripts={script: target}, history_cache={})
    backend = {"name": "fixture", "kind": "electrum", "url": "tcp://unused.invalid:1"}
    graph = {"txid": txid, "vin": [], "vout": [{"script_hex": script, "value_sats": 1000}]}
    with patch.object(sync_backends, "_electrum_client_context", side_effect=lambda _: nullcontext(Client())), patch.object(
        sync_backends, "decode_raw_transaction", return_value=graph,
    ):
        records, meta = sync_backends.compatibility_electrum_records_for_wallet(backend, state)
        initial_confirmation = records[0]["confirmed_at"]
        state = replace(state, checkpoint=meta["freshness_checkpoint"], history_cache={txid: records[0]["raw_json"]})
        assert sync_backends.compatibility_electrum_records_for_wallet(backend, state)[0] == []
        if tip_change == "legacy":
            state.checkpoint.pop("electrum_tip_fingerprint")
        else:
            tip = {"height": 100 + (tip_change == "advance"), "hex": "11" * 80}
            if tip_change == "missing":
                tip = None
            elif tip_change == "invalid":
                tip["hex"] = "invalid"
        timestamp += 600
        calls.clear()
        records, meta = sync_backends.compatibility_electrum_records_for_wallet(backend, state)
        assert len(records) == 1
        assert records[0]["confirmed_at"] == "2026-01-01T00:10:00Z"
        assert records[0]["confirmed_at"] != initial_confirmation
        assert meta["header_cache_hits"] == 0
        assert ("blockchain.block.header", [100]) in calls
        assert not any(method == "blockchain.transaction.get" for method, _ in calls)


@pytest.mark.parametrize("tip", [None, "", "²" * 64, "bad"])
def test_esplora_does_not_reuse_histories_without_a_valid_tip(tip):
    from tests.test_sync_backends import EsploraHistoryRetractionTest
    fixture = EsploraHistoryRetractionTest()
    tx = fixture.transaction(1, confirmed=True)
    _, initial, _ = fixture.scan({1: [tx]})
    state = WalletSyncState(chain="bitcoin", network="main", descriptor_plan=None,
        policy_asset_id="", targets=[fixture.target(1)],
        tracked_scripts={fixture.target(1)["script_pubkey"]: fixture.target(1)},
        history_cache={}, checkpoint=initial["freshness_checkpoint"])
    with patch.object(sync_backends, "http_get_text", return_value=tip or ""), patch.object(
        sync_backends, "esplora_scripthash_stats",
        return_value={"chain_stats": {"tx_count": 1}, "mempool_stats": {"tx_count": 0}},
    ), patch.object(sync_backends, "fetch_esplora_scripthash_transactions", return_value=[tx]) as history:
        records, _ = sync_backends.compatibility_esplora_records_for_wallet(
            {"name": "node", "kind": "esplora", "url": "https://node.example"}, state)
    assert len(records) == 1
    history.assert_called_once()


@pytest.mark.parametrize("scenario", ["chronology", "authority", "semantics"])
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

        if scenario == "semantics":
            conn.execute("UPDATE profiles SET journal_semantics_version=0 WHERE id='profile-1'")
            before = projection_freshness(conn, "profile-1")
            assert not before["is_current"]
            with patch("kassiber.core.sync._observed_accounting_inputs") as inputs:
                sync("wallet-a", 3)
            inputs.assert_not_called()
            assert projection_freshness(conn, "profile-1")["journal_input_version"] == before["journal_input_version"]
            return

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
