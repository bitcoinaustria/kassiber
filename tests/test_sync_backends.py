import unittest
from unittest.mock import patch

from kassiber.core.sync import WalletSyncHooks, WalletSyncState, sync_wallet_from_backend
from kassiber.core.sync_backends import (
    bitcoinrpc_sync_adapter,
    electrum_sync_adapter,
    esplora_sync_adapter,
    scriptpubkey_scripthash,
)
from kassiber.errors import AppError
from kassiber.time_utils import timestamp_to_iso


def _header_hex(timestamp):
    return ("00" * 68) + int(timestamp).to_bytes(4, "little").hex() + ("00" * 8)


class SyncBackendsTest(unittest.TestCase):
    def test_sync_wallet_from_backend_raises_for_unknown_backend_kind(self):
        wallet = {"label": "Watch", "config_json": "{}"}
        target = {"address": "bc1qwatch", "script_pubkey": "0014watch"}
        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {},
            resolve_backend=lambda runtime_config, backend_name: {
                "name": "custom",
                "kind": "custom",
                "url": "https://example.invalid",
            },
            resolve_sync_state=lambda backend, wallet: WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
            ),
            normalize_addresses=lambda values: [],
            backend_adapters={},
        )
        with self.assertRaises(AppError) as exc:
            sync_wallet_from_backend(None, {}, {}, wallet, hooks)
        self.assertIn("not implemented", str(exc.exception))

    def test_esplora_sync_adapter_returns_record_shape(self):
        target = {"address": "bc1qesplora", "script_pubkey": "0014esplora"}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        tx = {
            "txid": "11" * 32,
            "fee": 200,
            "vin": [],
            "vout": [{"scriptpubkey": target["script_pubkey"], "value": 12_345}],
            "status": {"block_time": 1_700_000_000},
        }
        with patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_transactions",
            return_value=[tx],
        ):
            records, meta = esplora_sync_adapter(
                {"name": "esplora", "kind": "esplora", "url": "https://esplora.example"},
                {"id": "wallet-1"},
                sync_state,
            )
        self.assertEqual(meta, {})
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], tx["txid"])
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(records[0]["asset"], "BTC")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.00012345, places=12)
        self.assertEqual(records[0]["occurred_at"], timestamp_to_iso(1_700_000_000))

    def test_electrum_sync_adapter_returns_record_shape(self):
        target = {"address": "bc1qe1", "script_pubkey": "0014deadbeef"}
        txid = "22" * 32
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        raw_map = {
            "current-raw": {
                "vin": [],
                "vout": [{"script_hex": target["script_pubkey"], "value_sats": 12_345}],
                "total_output_sats": 12_345,
            }
        }

        class FakeElectrumClient:
            def __init__(self, backend):
                self.backend = backend

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                responses = []
                for method, params in requests:
                    key = (method, tuple(params or ()))
                    if key == (
                        "blockchain.scripthash.get_history",
                        (scripthash,),
                    ):
                        responses.append([{"tx_hash": txid, "height": 123}])
                    elif key == ("blockchain.transaction.get", (txid,)):
                        responses.append("current-raw")
                    elif key == ("blockchain.block.header", (123,)):
                        responses.append(_header_hex(1_700_000_000))
                    else:
                        raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_raw_transaction",
            side_effect=lambda raw_hex: raw_map[raw_hex],
        ):
            records, meta = electrum_sync_adapter(
                {"name": "electrum", "kind": "electrum", "url": "ssl://electrum.example:50002"},
                {"id": "wallet-1"},
                sync_state,
            )
        self.assertEqual(meta, {})
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], txid)
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(records[0]["asset"], "BTC")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.00012345, places=12)
        self.assertEqual(records[0]["occurred_at"], timestamp_to_iso(1_700_000_000))

    def test_bitcoinrpc_sync_adapter_returns_record_and_meta_shape(self):
        target = {"address": "bc1qcore", "script_pubkey": "0014core"}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        wallet = {"id": "wallet-1"}

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None):
            del backend
            key = (method, tuple(params or ()), wallet_name)
            if key == ("listwallets", (), None):
                return []
            if key == ("loadwallet", ("kassiber-wallet-1", True), None):
                raise AppError("missing")
            if key == ("createwallet", ("kassiber-wallet-1", True, True, "", False, True, True), None):
                return {"name": "kassiber-wallet-1"}
            if key == ("getaddressinfo", ("bc1qcore",), "kassiber-wallet-1"):
                return {"iswatchonly": False, "ismine": False}
            if key == ("getdescriptorinfo", ("addr(bc1qcore)",), None):
                return {"descriptor": "addr(bc1qcore)#abcd"}
            if method == "importdescriptors" and wallet_name == "kassiber-wallet-1":
                self.assertEqual(
                    params,
                    [[{"desc": "addr(bc1qcore)#abcd", "timestamp": 0, "label": "kassiber:wallet-1"}]],
                )
                return [{"success": True}]
            if key == ("listtransactions", ("*", 1000, 0, True), "kassiber-wallet-1"):
                return [
                    {
                        "txid": "33" * 32,
                        "category": "receive",
                        "amount": 0.001,
                        "fee": 0,
                        "blocktime": 1_700_000_000,
                    }
                ]
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                wallet,
                sync_state,
            )
        self.assertEqual(meta["core_wallet"], "kassiber-wallet-1")
        self.assertEqual(meta["imported_addresses"], 1)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], "33" * 32)
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(records[0]["asset"], "BTC")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.001, places=12)
        self.assertEqual(records[0]["occurred_at"], timestamp_to_iso(1_700_000_000))


if __name__ == "__main__":
    unittest.main()
