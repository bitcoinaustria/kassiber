import io
import json
import unittest
from unittest.mock import patch

from kassiber.core.sync import (
    WalletSyncHooks,
    WalletSyncState,
    emit_sync_progress,
    sync_progress_emitter,
    sync_wallet_from_backend,
    sync_wallets,
)
from kassiber.core.sync_backends import (
    ElectrumClient,
    _connect_via_socks5,
    _emit_backend_progress,
    _read_exact,
    _socks5_address,
    bitcoinrpc_sync_adapter,
    discover_descriptor_targets,
    electrum_sync_adapter,
    esplora_sync_adapter,
    record_from_bitcoin_esplora_tx,
    record_from_bitcoinrpc_details,
    scan_descriptor_targets,
    scriptpubkey_scripthash,
)
from kassiber.errors import AppError
from kassiber.time_utils import timestamp_to_iso
from kassiber.wallet_descriptors import DescriptorBranch, DescriptorPlan, DerivedTarget


def _header_hex(timestamp):
    return ("00" * 68) + int(timestamp).to_bytes(4, "little").hex() + ("00" * 8)


class _DummySocket:
    def __init__(self):
        self.sent = []

    def sendall(self, payload):
        self.sent.append(payload)


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
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={},
        )
        with self.assertRaises(AppError) as exc:
            sync_wallet_from_backend(None, {}, {}, wallet, hooks)
        self.assertIn("not implemented", str(exc.exception))

    def test_sync_wallet_from_backend_attaches_wallet_to_backend_progress(self):
        wallet = {"label": "Cold", "config_json": "{}"}
        target = {"address": "bc1qwatch", "script_pubkey": "0014watch"}
        progress = []

        def adapter(backend, wallet, sync_state):
            emit_sync_progress({"phase": "backend_fetch", "known_txids": 2})
            return [], {"freshness_checkpoint": {"ok": True}}

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {"imported": 0, "skipped": 0},
            resolve_backend=lambda runtime_config, backend_name: {
                "name": "default",
                "kind": "esplora",
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
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": adapter},
        )

        token = sync_progress_emitter.set(lambda payload: progress.append(dict(payload)))
        try:
            sync_wallet_from_backend(None, {}, {}, wallet, hooks)
        finally:
            sync_progress_emitter.reset(token)

        self.assertTrue(progress)
        self.assertIn("discovery", [item.get("phase") for item in progress])
        self.assertIn("backend_fetch", [item.get("phase") for item in progress])
        self.assertEqual({item.get("wallet") for item in progress}, {"Cold"})
        self.assertEqual(progress[-1]["known_txids"], 2)

    def test_backend_progress_reuses_known_counters_for_ui_progress(self):
        progress = []
        token = sync_progress_emitter.set(lambda payload: progress.append(dict(payload)))
        try:
            _emit_backend_progress(
                "backend_fetch",
                target_count=10,
                targets_checked=3,
            )
            _emit_backend_progress(
                "decode_enrich",
                transactions_seen=40,
                transactions_total=100,
            )
        finally:
            sync_progress_emitter.reset(token)

        self.assertEqual(progress[0]["processed"], 3)
        self.assertEqual(progress[0]["total"], 10)
        self.assertEqual(progress[1]["processed"], 40)
        self.assertEqual(progress[1]["total"], 100)

    def test_sync_wallet_from_backend_keeps_inventory_when_utxos_skipped(self):
        wallet = {
            "id": "wallet-1",
            "label": "Watch",
            "kind": "descriptor",
            "config_json": '{"backend": "esplora", "addresses": ["bc1qwatch"]}',
        }
        profile = {"id": "profile-1"}
        target = {"address": "bc1qwatch", "script_pubkey": "0014watch"}
        inventory_calls = []

        def update_inventory(*args):
            inventory_calls.append(args)
            return {"updated": 1}

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {
                "imported": 0,
                "skipped": 0,
                "unchanged": 0,
                "journal_invalidated": False,
            },
            resolve_backend=lambda runtime_config, backend_name: {
                "name": backend_name,
                "kind": "esplora",
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
                checkpoint=wallet.get("_freshness_checkpoint"),
            ),
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={
                "esplora": lambda backend, wallet, sync_state: (
                    [],
                    {
                        "scripts_changed": 0,
                        "scripts_unchanged": 1,
                        "utxos_skipped_unchanged": True,
                    },
                )
            },
            update_output_inventory=update_inventory,
        )

        outcome = sync_wallet_from_backend(
            None,
            {},
            profile,
            wallet,
            hooks,
            checkpoint={"esplora_scripthashes": {}},
        )

        self.assertEqual(inventory_calls, [])
        self.assertFalse(outcome["utxos_refreshed"])
        self.assertTrue(outcome["utxos_skipped_unchanged"])
        self.assertEqual(outcome["scripts_checked"], 1)
        self.assertEqual(outcome["records_fetched"], 0)
        self.assertFalse(outcome["journal_invalidated"])
        self.assertIn("elapsed_ms", outcome)

    def test_sync_wallets_force_full_ignores_stored_checkpoint(self):
        wallet = {
            "id": "wallet-1",
            "kind": "descriptor",
            "label": "Watch",
            "config_json": json.dumps(
                {"backend": "esplora", "addresses": ["bc1qwatch"]}
            ),
        }
        profile = {"id": "profile-1"}
        target = {"address": "bc1qwatch", "script_pubkey": "0014watch"}
        checkpoints_seen = []

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {
                "imported": 0,
                "skipped": 0,
                "unchanged": 0,
                "journal_invalidated": False,
            },
            resolve_backend=lambda runtime_config, backend_name: {
                "name": backend_name,
                "kind": "esplora",
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
                checkpoint=wallet.get("_freshness_checkpoint"),
            ),
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={
                "esplora": lambda backend, wallet, sync_state: (
                    checkpoints_seen.append(sync_state.checkpoint) or [],
                    {"freshness_checkpoint": {"fresh": True}, "utxos": []},
                )
            },
        )

        results = sync_wallets(
            None,
            {},
            profile,
            [wallet],
            hooks,
            checkpoints={"wallet-1": {"highest_used": {"0": 42}}},
            force_full=True,
        )

        self.assertEqual(checkpoints_seen, [{}])
        self.assertTrue(results[0]["force_full"])
        self.assertTrue(results[0]["utxos_refreshed"])

    def test_esplora_sync_adapter_returns_record_shape(self):
        target = {"address": "bc1qesplora", "script_pubkey": "0014" + "11" * 20}
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
            "kassiber.core.sync_backends.esplora_scripthash_stats",
            return_value={"chain_stats": {"tx_count": 1}, "mempool_stats": {"tx_count": 0}},
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_transactions",
            return_value=[tx],
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_utxos",
            return_value=[],
        ), patch(
            "kassiber.core.sync_backends.http_get_text",
            return_value="123\n",
        ):
            records, meta = esplora_sync_adapter(
                {"name": "esplora", "kind": "esplora", "url": "https://esplora.example"},
                {"id": "wallet-1"},
                sync_state,
            )
        self.assertEqual(meta["scripts_changed"], 1)
        self.assertIn("freshness_checkpoint", meta)
        self.assertEqual(meta["utxos"], [])
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], tx["txid"])
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(records[0]["asset"], "BTC")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.00012345, places=12)
        self.assertEqual(records[0]["occurred_at"], timestamp_to_iso(1_700_000_000))
        self.assertEqual(records[0]["confirmed_at"], timestamp_to_iso(1_700_000_000))

    def test_esplora_checkpoint_skips_unchanged_script_pages(self):
        target = {"address": "bc1qesplora", "script_pubkey": "0014" + "11" * 20}
        tx = {
            "txid": "11" * 32,
            "fee": 200,
            "vin": [],
            "vout": [{"scriptpubkey": target["script_pubkey"], "value": 12_345}],
            "status": {"block_time": 1_700_000_000},
        }
        fetch_calls = []

        def fake_fetch(base_url, script_pubkey_hex, max_pages=None, timeout=30):
            fetch_calls.append((base_url, script_pubkey_hex, max_pages, timeout))
            return [tx]

        with patch(
            "kassiber.core.sync_backends.esplora_scripthash_stats",
            return_value={"chain_stats": {"tx_count": 1}, "mempool_stats": {"tx_count": 0}},
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_transactions",
            side_effect=fake_fetch,
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_utxos",
            return_value=[],
        ) as fetch_utxos:
            sync_state = WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
            )
            records, meta = esplora_sync_adapter(
                {"name": "esplora", "kind": "esplora", "url": "https://esplora.example"},
                {"id": "wallet-1"},
                sync_state,
            )
            self.assertEqual(len(records), 1)
            second_state = WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
                checkpoint=meta["freshness_checkpoint"],
            )
            records, second_meta = esplora_sync_adapter(
                {"name": "esplora", "kind": "esplora", "url": "https://esplora.example"},
                {"id": "wallet-1"},
                second_state,
            )

        self.assertEqual(records, [])
        self.assertEqual(second_meta["scripts_unchanged"], 1)
        self.assertTrue(second_meta["utxos_skipped_unchanged"])
        self.assertNotIn("utxos", second_meta)
        self.assertEqual(len(fetch_calls), 1)
        self.assertEqual(fetch_utxos.call_count, 1)

    def test_esplora_descriptor_discovery_rechecks_previously_unused_scripts(self):
        target = {"address": "bc1qgap", "script_pubkey": "0014" + "22" * 20}
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        usage_checks = []

        def fake_scan(
            plan,
            target_used=None,
            target_used_batch=None,
            scan_batch_size=None,
            highest_used=None,
        ):
            del plan, target_used, scan_batch_size, highest_used
            self.assertIsNotNone(target_used_batch)
            usage_checks.extend(target_used_batch([target]))
            return [target]

        with patch("kassiber.core.sync_backends.scan_descriptor_targets", side_effect=fake_scan), patch(
            "kassiber.core.sync_backends.esplora_scripthash_has_history",
            return_value=True,
        ) as has_history:
            discovery = discover_descriptor_targets(
                {"name": "esplora", "kind": "esplora", "url": "https://esplora.example"},
                object(),
                "esplora",
                checkpoint={"esplora_scripthashes": {scripthash: {"tx_count": 0}}},
            )

        self.assertEqual(discovery["targets"], [target])
        self.assertEqual(usage_checks, [True])
        has_history.assert_called_once_with(
            "https://esplora.example",
            target["script_pubkey"],
            timeout=30,
        )

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
                    if key == ("blockchain.scripthash.subscribe", (scripthash,)):
                        responses.append("status-1")
                    elif key == (
                        "blockchain.scripthash.get_history",
                        (scripthash,),
                    ):
                        responses.append([{"tx_hash": txid, "height": 123}])
                    elif key == (
                        "blockchain.scripthash.listunspent",
                        (scripthash,),
                    ):
                        responses.append(
                            [
                                {
                                    "tx_hash": txid,
                                    "tx_pos": 0,
                                    "height": 123,
                                    "value": 12_345,
                                }
                            ]
                        )
                    elif key == ("blockchain.transaction.get", (txid,)):
                        responses.append("current-raw")
                    elif key == ("blockchain.block.header", (123,)):
                        responses.append(_header_hex(1_700_000_000))
                    else:
                        raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses

            def call(self, method, params=None):
                key = (method, tuple(params or ()))
                if key == ("blockchain.headers.subscribe", ()):
                    return {"height": 125}
                raise AssertionError(f"Unexpected Electrum call: {key!r}")

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_raw_transaction",
            side_effect=lambda raw_hex: raw_map[raw_hex],
        ):
            records, meta = electrum_sync_adapter(
                {"name": "electrum", "kind": "electrum", "url": "ssl://electrum.example:50002"},
                {"id": "wallet-1"},
                sync_state,
            )
        self.assertEqual(meta["scripts_changed"], 1)
        self.assertIn("freshness_checkpoint", meta)
        self.assertEqual(len(meta["utxos"]), 1)
        self.assertEqual(meta["utxos"][0]["confirmations"], 3)
        self.assertEqual(meta["utxos"][0]["block_time"], timestamp_to_iso(1_700_000_000))
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], txid)
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(records[0]["asset"], "BTC")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.00012345, places=12)
        self.assertEqual(records[0]["occurred_at"], timestamp_to_iso(1_700_000_000))
        self.assertEqual(records[0]["confirmed_at"], timestamp_to_iso(1_700_000_000))

    def test_electrum_checkpoint_skips_unchanged_history_on_second_sync(self):
        target = {"address": "bc1qe1", "script_pubkey": "0014deadbeef"}
        txid = "22" * 32
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        raw_map = {
            "current-raw": {
                "vin": [],
                "vout": [{"script_hex": target["script_pubkey"], "value_sats": 12_345}],
                "total_output_sats": 12_345,
            }
        }
        calls = []

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
                    calls.append(key)
                    if key == ("blockchain.scripthash.subscribe", (scripthash,)):
                        responses.append("status-1")
                    elif key == ("blockchain.scripthash.get_history", (scripthash,)):
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
        ), patch(
            "kassiber.core.sync_backends.electrum_utxos_for_wallet",
            return_value=[],
        ) as fetch_utxos:
            sync_state = WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
            )
            records, meta = electrum_sync_adapter(
                {"name": "electrum", "kind": "electrum", "url": "ssl://electrum.example:50002"},
                {"id": "wallet-1"},
                sync_state,
            )
            self.assertEqual(len(records), 1)
            first_call_count = len(calls)
            second_state = WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
                checkpoint=meta["freshness_checkpoint"],
            )
            records, second_meta = electrum_sync_adapter(
                {"name": "electrum", "kind": "electrum", "url": "ssl://electrum.example:50002"},
                {"id": "wallet-1"},
                second_state,
            )

        second_calls = calls[first_call_count:]
        self.assertEqual(records, [])
        self.assertEqual(second_meta["scripts_unchanged"], 1)
        self.assertTrue(second_meta["utxos_skipped_unchanged"])
        self.assertNotIn("utxos", second_meta)
        self.assertEqual(fetch_utxos.call_count, 1)
        self.assertEqual(
            second_calls,
            [("blockchain.scripthash.subscribe", (scripthash,))],
        )

    def test_electrum_descriptor_discovery_rechecks_cached_unused_status(self):
        target = {"address": "bc1qgap", "script_pubkey": "0014cafebabe"}
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        batch_calls = []

        class FakeElectrumClient:
            def __init__(self, backend):
                self.backend = backend

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                batch_calls.extend(requests)
                responses = []
                for method, params in requests:
                    key = (method, tuple(params or ()))
                    if key == ("blockchain.scripthash.subscribe", (scripthash,)):
                        responses.append("status-new")
                    else:
                        raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses

        def fake_scan(
            plan,
            target_used=None,
            target_used_batch=None,
            scan_batch_size=None,
            highest_used=None,
        ):
            del plan, target_used, scan_batch_size, highest_used
            self.assertIsNotNone(target_used_batch)
            self.assertEqual(target_used_batch([target]), [True])
            return [target]

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.scan_descriptor_targets",
            side_effect=fake_scan,
        ):
            discovery = discover_descriptor_targets(
                {
                    "name": "electrum",
                    "kind": "electrum",
                    "url": "ssl://electrum.example:50002",
                    "batch_size": 10,
                },
                object(),
                "electrum",
                checkpoint={"electrum_scripthash_statuses": {scripthash: None}},
            )

        self.assertEqual(discovery["targets"], [target])
        self.assertEqual(
            batch_calls,
            [("blockchain.scripthash.subscribe", [scripthash])],
        )

    def test_descriptor_scan_reuses_highest_used_targets_and_checks_trailing_gap(self):
        class FakeDescriptor:
            is_wildcard = True

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=2,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(0, "receive", FakeDescriptor()),),
        )
        checked = []

        def fake_derive(plan, branch_index=None, start=0, end=0):
            del plan
            return [
                DerivedTarget(
                    chain="bitcoin",
                    network="bitcoin",
                    branch_index=branch_index,
                    branch_label="receive",
                    address_index=index,
                    address=f"bc1q{index}",
                    unconfidential_address=None,
                    script_pubkey=f"{index:064x}",
                    derivation_path=f"m/0/{index}",
                    derivation_paths=(f"m/0/{index}",),
                    key_origins=(),
                )
                for index in range(start, end)
            ]

        def target_used_batch(targets):
            checked.extend(target["address_index"] for target in targets)
            return [False for _ in targets]

        with patch(
            "kassiber.core.sync_backends.derive_descriptor_targets",
            side_effect=fake_derive,
        ):
            targets = scan_descriptor_targets(
                plan,
                target_used_batch=target_used_batch,
                scan_batch_size=1,
                highest_used={"0": 2},
            )

        self.assertEqual([target["address_index"] for target in targets], [0, 1, 2, 3, 4])
        self.assertEqual(checked, [3, 4])

    def test_electrum_call_raises_app_error_for_non_json_response(self):
        client = ElectrumClient({"name": "electrum", "url": "tcp://electrum.example:50001"})
        client.socket = _DummySocket()
        client.reader = io.StringIO("<html>not electrum</html>\n")

        with self.assertRaises(AppError) as raised:
            client.call("server.version", [])

        self.assertIn("Electrum-format JSON", str(raised.exception))
        self.assertEqual(
            raised.exception.hint,
            "Check that the backend URL points to an Electrum server and uses the correct tcp/ssl port.",
        )
        self.assertEqual(
            raised.exception.details,
            {"response_preview": "<html>not electrum</html>"},
        )
        self.assertTrue(raised.exception.retryable)

    def test_electrum_batch_call_raises_app_error_for_non_json_response(self):
        client = ElectrumClient({"name": "electrum", "url": "tcp://electrum.example:50001"})
        client.socket = _DummySocket()
        client.reader = io.StringIO("not json\n")

        with self.assertRaises(AppError) as raised:
            client.batch_call([("server.version", [])])

        self.assertIn("Electrum-format JSON", str(raised.exception))
        self.assertEqual(raised.exception.details, {"response_preview": "not json"})

    def test_electrum_call_raises_app_error_for_non_object_json_response(self):
        client = ElectrumClient({"name": "electrum", "url": "tcp://electrum.example:50001"})
        client.socket = _DummySocket()
        client.reader = io.StringIO("[]\n")

        with self.assertRaises(AppError) as raised:
            client.call("server.version", [])

        self.assertIn("Electrum-format JSON", str(raised.exception))
        self.assertEqual(raised.exception.details, {"response_type": "list"})

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
            if key == ("getbestblockhash", (), None):
                return "aa" * 32
            if key == (
                "listunspent",
                (0, 9999999, ["bc1qcore"], True),
                "kassiber-wallet-1",
            ):
                return []
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                wallet,
                sync_state,
            )
        self.assertEqual(meta["core_wallet"], "kassiber-wallet-1")
        self.assertEqual(meta["imported_addresses"], 1)
        self.assertEqual(meta["bitcoinrpc_sync_mode"], "full_scan")
        self.assertEqual(meta["freshness_checkpoint"]["bitcoinrpc_last_block"], "aa" * 32)
        self.assertEqual(meta["utxos"], [])
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], "33" * 32)
        self.assertEqual(records[0]["direction"], "inbound")
        self.assertEqual(records[0]["asset"], "BTC")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.001, places=12)
        self.assertEqual(records[0]["occurred_at"], timestamp_to_iso(1_700_000_000))
        self.assertEqual(records[0]["confirmed_at"], timestamp_to_iso(1_700_000_000))

    def test_bitcoinrpc_checkpoint_uses_listsinceblock(self):
        target = {"address": "bc1qcore", "script_pubkey": "0014core"}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
            checkpoint={"bitcoinrpc_last_block": "aa" * 32},
        )
        wallet = {"id": "wallet-1"}
        calls = []

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None):
            del backend
            key = (method, tuple(params or ()), wallet_name)
            calls.append(method)
            if key == ("listwallets", (), None):
                return ["kassiber-wallet-1"]
            if key == ("getaddressinfo", ("bc1qcore",), "kassiber-wallet-1"):
                return {"iswatchonly": True, "ismine": False}
            if key == ("listsinceblock", ("aa" * 32, 1, True, True), "kassiber-wallet-1"):
                return {
                    "transactions": [
                        {
                            "txid": "44" * 32,
                            "category": "receive",
                            "amount": 0.002,
                            "fee": 0,
                            "blocktime": 1_700_000_100,
                        }
                    ],
                    "lastblock": "bb" * 32,
                    "removed": [],
                }
            if key == (
                "listunspent",
                (0, 9999999, ["bc1qcore"], True),
                "kassiber-wallet-1",
            ):
                return []
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                wallet,
                sync_state,
            )

        self.assertNotIn("listtransactions", calls)
        self.assertEqual(meta["imported_addresses"], 0)
        self.assertEqual(meta["bitcoinrpc_sync_mode"], "sinceblock")
        self.assertEqual(meta["freshness_checkpoint"]["bitcoinrpc_last_block"], "bb" * 32)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], "44" * 32)

    def test_esplora_mempool_record_leaves_confirmed_at_empty(self):
        tracked_script = "0014watch"
        tx = {
            "txid": "44" * 32,
            "fee": 100,
            "vin": [],
            "vout": [{"scriptpubkey": tracked_script, "value": 10_000}],
            "status": {"confirmed": False},
        }
        record = record_from_bitcoin_esplora_tx(tx, {tracked_script: {"address": "bc1qwatch"}}, "esplora")
        self.assertEqual(record["occurred_at"], timestamp_to_iso(None))
        self.assertIsNone(record["confirmed_at"])

    def test_bitcoinrpc_unconfirmed_record_leaves_confirmed_at_empty(self):
        record = record_from_bitcoinrpc_details(
            "55" * 32,
            [{"category": "receive", "amount": 0.001, "fee": 0, "time": 1_700_000_000}],
            "core",
        )
        self.assertEqual(record["occurred_at"], timestamp_to_iso(1_700_000_000))
        self.assertIsNone(record["confirmed_at"])


class _FakeSocket:
    """In-memory socket double for SOCKS5 protocol tests."""

    def __init__(self, responses):
        self.sent = bytearray()
        self._inbox = bytearray(b"".join(responses))
        self.closed = False

    def sendall(self, data):
        self.sent.extend(data)

    def recv(self, length):
        chunk = bytes(self._inbox[:length])
        del self._inbox[:length]
        return chunk

    def close(self):
        self.closed = True


class Socks5HelpersTest(unittest.TestCase):
    def test_socks5_address_ipv4(self):
        self.assertEqual(_socks5_address("192.0.2.1"), b"\x01\xc0\x00\x02\x01")

    def test_socks5_address_ipv6(self):
        self.assertEqual(_socks5_address("::1"), b"\x04" + (b"\x00" * 15) + b"\x01")

    def test_socks5_address_domain(self):
        self.assertEqual(
            _socks5_address("node.example"),
            b"\x03" + bytes([len("node.example")]) + b"node.example",
        )

    def test_socks5_address_rejects_oversized_host(self):
        # Five 60-byte labels separated by dots is 304 bytes after IDNA encoding,
        # which clears the per-label 63-byte limit but trips the >255 total guard.
        oversized = ".".join(["a" * 60] * 5)
        with self.assertRaises(AppError):
            _socks5_address(oversized)

    def test_socks5_address_rejects_invalid_label(self):
        # A single 256-byte label is rejected by the IDNA codec before the
        # length check; the helper should still surface that as an AppError.
        with self.assertRaises(AppError):
            _socks5_address("a" * 256 + ".example")

    def test_read_exact_assembles_across_chunks(self):
        sock = _FakeSocket([b"ab", b"cd", b"ef"])
        self.assertEqual(_read_exact(sock, 6), b"abcdef")

    def test_read_exact_raises_on_early_close(self):
        sock = _FakeSocket([b"ab", b""])
        with self.assertRaises(AppError):
            _read_exact(sock, 6)

    def test_connect_via_socks5_happy_path(self):
        fake = _FakeSocket(
            [
                b"\x05\x00",  # auth accepted (no-auth)
                # response: ver, status=success, rsv, atyp=ipv4, bound ip + port
                b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00",
            ],
        )
        with patch(
            "kassiber.core.sync_backends.socket.create_connection",
            return_value=fake,
        ):
            sock = _connect_via_socks5(
                "socks5://127.0.0.1:9050",
                "node.example",
                50002,
                timeout=5,
            )
        self.assertIs(sock, fake)
        self.assertFalse(fake.closed)
        sent = bytes(fake.sent)
        self.assertEqual(sent[:3], b"\x05\x01\x00")  # greeting
        expected_request = (
            b"\x05\x01\x00\x03"
            + bytes([len("node.example")])
            + b"node.example"
            + (50002).to_bytes(2, "big")
        )
        self.assertIn(expected_request, sent)

    def test_connect_via_socks5_rejects_authenticated_proxy(self):
        fake = _FakeSocket([b"\x05\x02"])  # proxy requires user/pass
        with patch(
            "kassiber.core.sync_backends.socket.create_connection",
            return_value=fake,
        ):
            with self.assertRaisesRegex(AppError, "0x02"):
                _connect_via_socks5(
                    "socks5://127.0.0.1:9050",
                    "node.example",
                    50002,
                    timeout=5,
                )
        self.assertTrue(fake.closed)

    def test_connect_via_socks5_rejects_non_socks5_greeting(self):
        # A SOCKS4 / HTTP proxy will not start its reply with 0x05; the helper
        # should call that out instead of reporting an auth-method mismatch.
        fake = _FakeSocket([b"\x04\x5a"])
        with patch(
            "kassiber.core.sync_backends.socket.create_connection",
            return_value=fake,
        ):
            with self.assertRaisesRegex(AppError, "unexpected greeting"):
                _connect_via_socks5(
                    "socks5://127.0.0.1:9050",
                    "node.example",
                    50002,
                    timeout=5,
                )
        self.assertTrue(fake.closed)

    def test_connect_via_socks5_surfaces_connect_failure(self):
        fake = _FakeSocket(
            [
                b"\x05\x00",
                b"\x05\x05\x00\x01\x00\x00\x00\x00\x00\x00",  # status=5 (connection refused)
            ],
        )
        with patch(
            "kassiber.core.sync_backends.socket.create_connection",
            return_value=fake,
        ):
            with self.assertRaises(AppError):
                _connect_via_socks5(
                    "socks5://127.0.0.1:9050",
                    "node.example",
                    50002,
                    timeout=5,
                )
        self.assertTrue(fake.closed)


if __name__ == "__main__":
    unittest.main()
