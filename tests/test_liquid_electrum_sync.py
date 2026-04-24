import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core.sync import WalletSyncState
from kassiber.core.sync_backends import (
    discover_descriptor_targets,
    electrum_records_for_wallet,
    record_components_from_liquid_tx,
    scriptpubkey_scripthash,
    validate_backend_for_wallet,
)
from kassiber.core.wallets import wallet_policy_asset_id
from kassiber.db import open_db
from kassiber.time_utils import timestamp_to_iso
from kassiber.wallet_descriptors import default_policy_asset_id


class _HexBytes:
    def __init__(self, value):
        self._value = value

    def hex(self):
        return self._value


class _ScriptPubKey:
    def __init__(self, script_hex):
        self.data = _HexBytes(script_hex)


class _FakeOutput:
    def __init__(self, script_hex, value_sats, asset_id):
        self.script_pubkey = _ScriptPubKey(script_hex)
        self.fake_value_sats = value_sats
        self.fake_asset_id = asset_id


class _FakeInput:
    def __init__(self, txid_hex, vout):
        self.txid = bytes.fromhex(txid_hex)
        self.vout = vout


class _FakeTx:
    def __init__(self, vin, vout):
        self.vin = vin
        self.vout = vout


def _header_hex(timestamp):
    return ("00" * 68) + int(timestamp).to_bytes(4, "little").hex() + ("00" * 8)


class LiquidElectrumSyncTest(unittest.TestCase):
    def test_validate_backend_allows_liquid_electrum(self):
        backend = {
            "name": "liquid",
            "kind": "electrum",
            "chain": "liquid",
            "network": "liquidv1",
            "url": "ssl://liquid.example:995",
        }
        kind = validate_backend_for_wallet(backend, "liquid", "liquidv1", has_descriptor=True)
        self.assertEqual(kind, "electrum")

    def test_record_components_from_liquid_tx_tracks_prevouts_and_fee(self):
        policy_asset_id = default_policy_asset_id("liquidv1")
        tracked_script = "0014feedface"
        current_tx = _FakeTx(
            vin=[_FakeInput("11" * 32, 0)],
            vout=[
                _FakeOutput("", 19, policy_asset_id),
                _FakeOutput("51", 500, policy_asset_id),
            ],
        )
        prev_tx = _FakeTx(vin=[], vout=[_FakeOutput(tracked_script, 1000, policy_asset_id)])

        with patch(
            "kassiber.core.sync_backends.liquid_output_amount_asset_id",
            side_effect=lambda output, plan, target=None: (output.fake_value_sats, output.fake_asset_id),
        ):
            records = record_components_from_liquid_tx(
                txid="aa" * 32,
                occurred_at="2026-01-01T00:00:00Z",
                tx=current_tx,
                descriptor_plan=object(),
                tracked_scripts={
                    tracked_script: {
                        "branch_index": 0,
                        "address_index": 0,
                        "script_pubkey": tracked_script,
                        "address": "lq1test",
                    }
                },
                backend_name="liquid",
                policy_asset_id=policy_asset_id,
                prev_tx_lookup=lambda txid: prev_tx,
                raw_json_context={"source": "unit-test"},
            )

        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["direction"], "outbound")
        self.assertEqual(record["asset"], "LBTC")
        self.assertAlmostEqual(float(record["amount"]), 0.00000981, places=12)
        self.assertAlmostEqual(float(record["fee"]), 0.00000019, places=12)
        self.assertEqual(record["kind"], "withdrawal")
        payload = json.loads(record["raw_json"])
        self.assertEqual(payload["txid"], "aa" * 32)
        self.assertEqual(payload["source"], "unit-test")
        self.assertEqual(payload["component"]["net_sats"], -1000)
        self.assertEqual(payload["component"]["fee_sats"], 19)

    def test_electrum_records_for_liquid_wallet(self):
        policy_asset_id = default_policy_asset_id("liquidv1")
        tracked_script = "0014c54c073c10cf177cf5157e0861757586f4029b96"
        target = {
            "branch_index": 0,
            "branch_label": "receive",
            "address_index": 0,
            "address": "lq1test",
            "script_pubkey": tracked_script,
        }
        current_txid = "c1" * 32
        prev_txid = "ea" * 32
        current_tx = _FakeTx(
            vin=[_FakeInput(prev_txid, 0)],
            vout=[
                _FakeOutput("", 19, policy_asset_id),
                _FakeOutput(tracked_script, 20901, policy_asset_id),
            ],
        )
        prev_tx = _FakeTx(vin=[], vout=[_FakeOutput("51", 50000, policy_asset_id)])
        raw_map = {
            "current-raw": current_tx,
            "prev-raw": prev_tx,
        }
        calls = []
        history = [{"tx_hash": current_txid, "height": 123}]
        responses = {
            ("blockchain.scripthash.get_history", (scriptpubkey_scripthash(tracked_script),)): history,
            ("blockchain.transaction.get", (current_txid,)): "current-raw",
            ("blockchain.transaction.get", (prev_txid,)): "prev-raw",
            ("blockchain.block.header", (123,)): _header_hex(1_700_000_000),
        }

        class FakeElectrumClient:
            def __init__(self, backend):
                self.backend = backend

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def call(self, method, params=None):
                key = (method, tuple(params or ()))
                calls.append(key)
                if key not in responses:
                    raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses[key]

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_liquid_transaction",
            side_effect=lambda raw_hex: raw_map[raw_hex],
        ), patch(
            "kassiber.core.sync_backends.liquid_output_amount_asset_id",
            side_effect=lambda output, plan, target=None: (output.fake_value_sats, output.fake_asset_id),
        ):
            records = electrum_records_for_wallet(
                {"name": "liquid", "kind": "electrum", "url": "ssl://liquid.example:995"},
                WalletSyncState(
                    chain="liquid",
                    network="liquidv1",
                    descriptor_plan=object(),
                    policy_asset_id=policy_asset_id,
                    targets=[target],
                    tracked_scripts={tracked_script: target},
                    history_cache={},
                ),
            )

        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["direction"], "inbound")
        self.assertEqual(record["asset"], "LBTC")
        self.assertAlmostEqual(float(record["amount"]), 0.00020901, places=12)
        self.assertAlmostEqual(float(record["fee"]), 0.0, places=12)
        self.assertEqual(record["occurred_at"], timestamp_to_iso(1_700_000_000))
        self.assertIn(("blockchain.transaction.get", (current_txid,)), calls)
        self.assertIn(("blockchain.transaction.get", (prev_txid,)), calls)
        self.assertIn(("blockchain.block.header", (123,)), calls)

    def test_discover_descriptor_targets_reuses_history_cache_with_backend_batch_size(self):
        first_target = {"script_pubkey": "0014feedface"}
        second_target = {"script_pubkey": "0014deadbeef"}
        first_hash = scriptpubkey_scripthash(first_target["script_pubkey"])
        second_hash = scriptpubkey_scripthash(second_target["script_pubkey"])
        batch_calls = []

        class FakeElectrumClient:
            def __init__(self, backend):
                self.backend = backend

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                batch_calls.append(requests)
                responses = []
                for method, params in requests:
                    key = (method, tuple(params or ()))
                    if key == ("blockchain.scripthash.get_history", (first_hash,)):
                        responses.append([{"tx_hash": "11" * 32, "height": 7}])
                    elif key == ("blockchain.scripthash.get_history", (second_hash,)):
                        responses.append([])
                    else:
                        raise AssertionError(f"Unexpected batched call: {key!r}")
                return responses

        def fake_scan(plan, target_used=None, target_used_batch=None, scan_batch_size=None):
            self.assertIsNone(target_used)
            self.assertEqual(scan_batch_size, 1)
            self.assertEqual(target_used_batch([first_target, second_target]), [True, False])
            return [first_target, second_target]

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.scan_descriptor_targets",
            side_effect=fake_scan,
        ):
            discovery = discover_descriptor_targets(
                {
                    "name": "liquid",
                    "kind": "electrum",
                    "url": "ssl://liquid.example:995",
                    "batch_size": 1,
                },
                object(),
                "electrum",
            )

        self.assertEqual(discovery["targets"], [first_target, second_target])
        self.assertEqual(
            batch_calls,
            [
                [("blockchain.scripthash.get_history", [first_hash])],
                [("blockchain.scripthash.get_history", [second_hash])],
            ],
        )
        self.assertEqual(discovery["history_cache"][first_hash], [{"tx_hash": "11" * 32, "height": 7}])
        self.assertEqual(discovery["history_cache"][second_hash], [])

    def test_electrum_records_reuse_history_cache_and_batch_fetches(self):
        policy_asset_id = default_policy_asset_id("liquidv1")
        tracked_script = "0014c54c073c10cf177cf5157e0861757586f4029b96"
        target = {
            "branch_index": 0,
            "branch_label": "receive",
            "address_index": 0,
            "address": "lq1test",
            "script_pubkey": tracked_script,
        }
        current_txid = "c1" * 32
        prev_txid = "ea" * 32
        current_tx = _FakeTx(
            vin=[_FakeInput(prev_txid, 0)],
            vout=[
                _FakeOutput("", 19, policy_asset_id),
                _FakeOutput(tracked_script, 20901, policy_asset_id),
            ],
        )
        prev_tx = _FakeTx(vin=[], vout=[_FakeOutput("51", 50000, policy_asset_id)])
        raw_map = {
            "current-raw": current_tx,
            "prev-raw": prev_tx,
        }
        batched_requests = []

        class FakeElectrumClient:
            def __init__(self, backend):
                self.backend = backend

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def call(self, method, params=None):
                raise AssertionError(f"Unexpected non-batched call: {(method, tuple(params or ()))}")

            def batch_call(self, requests):
                batched_requests.append(requests)
                responses = []
                for method, params in requests:
                    key = (method, tuple(params or ()))
                    if key == ("blockchain.transaction.get", (current_txid,)):
                        responses.append("current-raw")
                    elif key == ("blockchain.transaction.get", (prev_txid,)):
                        responses.append("prev-raw")
                    elif key == ("blockchain.block.header", (123,)):
                        responses.append(_header_hex(1_700_000_000))
                    else:
                        raise AssertionError(f"Unexpected batched call: {key!r}")
                return responses

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_liquid_transaction",
            side_effect=lambda raw_hex: raw_map[raw_hex],
        ), patch(
            "kassiber.core.sync_backends.liquid_output_amount_asset_id",
            side_effect=lambda output, plan, target=None: (output.fake_value_sats, output.fake_asset_id),
        ):
            records = electrum_records_for_wallet(
                {"name": "liquid", "kind": "electrum", "url": "ssl://liquid.example:995"},
                WalletSyncState(
                    chain="liquid",
                    network="liquidv1",
                    descriptor_plan=object(),
                    policy_asset_id=policy_asset_id,
                    targets=[target],
                    tracked_scripts={tracked_script: target},
                    history_cache={
                        scriptpubkey_scripthash(tracked_script): [{"tx_hash": current_txid, "height": 123}]
                    },
                ),
            )

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(
            batched_requests,
            [
                [("blockchain.transaction.get", [current_txid])],
                [("blockchain.transaction.get", [prev_txid])],
                [("blockchain.block.header", [123])],
            ],
        )


class LiquidPolicyAssetResolutionTest(unittest.TestCase):
    def test_symbolic_lbtc_resolves_to_network_hex_on_liquid(self):
        hex_id = default_policy_asset_id("liquidv1")
        for symbolic in ("L-BTC", "l-btc", "LBTC", "lbtc"):
            with self.subTest(symbolic=symbolic):
                resolved = wallet_policy_asset_id(
                    {"policy_asset": symbolic}, "liquid", "liquidv1"
                )
                self.assertEqual(resolved, hex_id)

    def test_explicit_hex_policy_asset_is_preserved(self):
        hex_id = default_policy_asset_id("liquidv1")
        resolved = wallet_policy_asset_id(
            {"policy_asset": hex_id}, "liquid", "liquidv1"
        )
        self.assertEqual(resolved, hex_id)

    def test_symbolic_policy_asset_on_non_liquid_chain_is_left_alone(self):
        resolved = wallet_policy_asset_id(
            {"policy_asset": "LBTC"}, "bitcoin", "main"
        )
        self.assertEqual(resolved, "LBTC")

    def test_missing_policy_asset_falls_back_to_network_hex(self):
        hex_id = default_policy_asset_id("liquidv1")
        resolved = wallet_policy_asset_id({}, "liquid", "liquidv1")
        self.assertEqual(resolved, hex_id)


class LiquidAssetBackfillTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-liquid-asset-backfill-")
        self.addCleanup(self._tmp.cleanup)
        self.data_root = Path(self._tmp.name) / "data"

    def _seed_minimal_schema(self, conn):
        now = "2026-04-20T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces (id, label, created_at) VALUES (?, ?, ?)",
            ("ws-1", "ws", now),
        )
        conn.execute(
            "INSERT INTO profiles (id, workspace_id, label, created_at, last_processed_at, last_processed_tx_count) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            ("prof-1", "ws-1", "main", now, "2026-04-21T00:00:00Z", 12),
        )
        conn.execute(
            "INSERT INTO wallets (id, workspace_id, profile_id, label, kind, config_json, created_at) "
            "VALUES (?, ?, ?, ?, ?, ?, ?)",
            ("wal-1", "ws-1", "prof-1", "Liquid", "descriptor", "{}", now),
        )
        conn.commit()

    def _insert_tx(self, conn, tx_id, asset):
        conn.execute(
            "INSERT INTO transactions ("
            "id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at, "
            "direction, asset, amount, fee, created_at"
            ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                tx_id,
                "ws-1",
                "prof-1",
                "wal-1",
                f"fp-{tx_id}",
                "2026-04-15T12:00:00Z",
                "inbound",
                asset,
                1_000,
                0,
                "2026-04-15T12:00:00Z",
            ),
        )
        conn.commit()

    def test_open_db_rewrites_liquid_hex_to_lbtc(self):
        hex_id = default_policy_asset_id("liquidv1")
        conn = open_db(str(self.data_root))
        try:
            self._seed_minimal_schema(conn)
            self._insert_tx(conn, "tx-hex", hex_id)
            self._insert_tx(conn, "tx-btc", "BTC")
        finally:
            conn.close()

        # Re-open to trigger ensure_schema_compat → _backfill_liquid_asset_codes.
        conn = open_db(str(self.data_root))
        try:
            assets = {
                row["id"]: row["asset"]
                for row in conn.execute("SELECT id, asset FROM transactions").fetchall()
            }
            self.assertEqual(assets["tx-hex"], "LBTC")
            self.assertEqual(assets["tx-btc"], "BTC")
            profile = conn.execute(
                "SELECT last_processed_at, last_processed_tx_count FROM profiles WHERE id = ?",
                ("prof-1",),
            ).fetchone()
            self.assertIsNone(profile["last_processed_at"])
            self.assertEqual(profile["last_processed_tx_count"], 0)
        finally:
            conn.close()

    def test_backfill_is_idempotent(self):
        conn = open_db(str(self.data_root))
        try:
            self._seed_minimal_schema(conn)
            self._insert_tx(conn, "tx-btc", "BTC")
        finally:
            conn.close()

        # Second open: no hex rows, profile should stay as-is.
        conn = open_db(str(self.data_root))
        try:
            profile = conn.execute(
                "SELECT last_processed_at, last_processed_tx_count FROM profiles WHERE id = ?",
                ("prof-1",),
            ).fetchone()
            self.assertEqual(profile["last_processed_at"], "2026-04-21T00:00:00Z")
            self.assertEqual(profile["last_processed_tx_count"], 12)
        finally:
            conn.close()


if __name__ == "__main__":
    unittest.main()
