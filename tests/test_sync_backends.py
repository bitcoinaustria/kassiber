import io
import json
import random
import sqlite3
import unittest
from email.message import Message
from types import SimpleNamespace
from unittest.mock import patch
from urllib import error as urlerror

from kassiber.core import sync_backends as sb
from kassiber.core import wallets as core_wallets
from kassiber.core.sync import (
    WalletBackendFetch,
    WalletSyncHooks,
    WalletSyncState,
    NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT,
    classify_wallet_sync,
    emit_sync_progress,
    fetch_wallet_backend,
    prefetch_wallets_backend,
    sync_progress_emitter,
    sync_wallet_from_backend,
    sync_wallets,
)
from kassiber.core.sync_backends import (
    ElectrumClient,
    _connect_backend_socket,
    _emit_backend_progress,
    bitcoinrpc_import_ranged_descriptors,
    bitcoinrpc_sync_adapter,
    discover_descriptor_targets,
    electrum_sync_adapter,
    esplora_sync_adapter,
    esplora_utxos_for_wallet,
    record_from_bitcoin_esplora_tx,
    record_from_bitcoinrpc_details,
    scan_descriptor_targets,
    scriptpubkey_scripthash,
)
from kassiber.errors import AppError
from kassiber.proxy import _connect_via_socks5, _read_exact, _socks5_address
from kassiber.time_utils import iso_to_unix, timestamp_to_iso
from kassiber.wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    DescriptorBranch,
    DescriptorPlan,
    DerivedTarget,
)


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

    def test_sync_wallet_from_backend_wraps_unexpected_backend_shape(self):
        wallet = {"label": "Cold", "config_json": "{}"}
        target = {"address": "bc1qwatch", "script_pubkey": "0014watch"}

        def adapter(backend, wallet, sync_state):
            raise ValueError("invalid literal for int() with base 10: '2026-04-14T10:17:10Z'")

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {},
            resolve_backend=lambda runtime_config, backend_name: {
                "name": "liquid",
                "kind": "electrum",
                "chain": "liquid",
                "network": "liquidv1",
                "url": "ssl://liquid.example:995",
            },
            resolve_sync_state=lambda backend, wallet: WalletSyncState(
                chain="liquid",
                network="liquidv1",
                descriptor_plan=object(),
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
            ),
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"electrum": adapter},
        )

        with self.assertRaises(AppError) as exc:
            sync_wallet_from_backend(None, {}, {}, wallet, hooks)

        self.assertEqual(exc.exception.code, "backend_sync_failed")
        self.assertTrue(exc.exception.retryable)
        self.assertIn("Cold", str(exc.exception))
        self.assertEqual(exc.exception.details["wallet"], "Cold")
        self.assertEqual(exc.exception.details["backend"], "liquid")
        self.assertEqual(exc.exception.details["phase"], "backend_fetch")
        self.assertEqual(exc.exception.details["error_type"], "ValueError")
        self.assertTrue(exc.exception.details["has_backend_url"])

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

    def test_backend_progress_allows_scanned_count_before_total_is_known(self):
        progress = []
        token = sync_progress_emitter.set(lambda payload: progress.append(dict(payload)))
        try:
            _emit_backend_progress("discovery", targets_checked=150)
        finally:
            sync_progress_emitter.reset(token)

        self.assertEqual(progress[0]["phase"], "discovery")
        self.assertEqual(progress[0]["processed"], 150)
        self.assertNotIn("total", progress[0])

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

    def test_sync_wallet_from_backend_repairs_negative_running_balance_with_widened_rescan(self):
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE transactions (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                wallet_id TEXT NOT NULL,
                external_id TEXT,
                occurred_at TEXT NOT NULL,
                direction TEXT NOT NULL,
                asset TEXT NOT NULL,
                amount INTEGER NOT NULL,
                fee INTEGER NOT NULL DEFAULT 0,
                excluded INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            )
            """
        )
        wallet = {
            "id": "wallet-1",
            "kind": "descriptor",
            "label": "Vault",
            "config_json": json.dumps({"backend": "default", "descriptor": "dummy"}),
        }
        profile = {"id": "profile-1"}
        target = {
            "address": "bc1qwatch",
            "script_pubkey": "0014watch",
            "branch_index": 0,
            "address_index": 0,
        }
        adapter_gaps = []
        checkpoints_seen = []

        def resolve_sync_state(backend, wallet_row):
            config = json.loads(wallet_row["config_json"] or "{}")
            gap_limit = int(config.get("gap_limit") or DEFAULT_DESCRIPTOR_GAP_LIMIT)
            checkpoints_seen.append(wallet_row.get("_freshness_checkpoint"))
            return WalletSyncState(
                chain="bitcoin",
                network="main",
                descriptor_plan=SimpleNamespace(gap_limit=gap_limit),
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
                checkpoint=wallet_row.get("_freshness_checkpoint"),
            )

        def adapter(backend, wallet_row, sync_state):
            adapter_gaps.append(sync_state.descriptor_plan.gap_limit)
            return (
                [{"pass": len(adapter_gaps)}],
                {"freshness_checkpoint": {"pass": len(adapter_gaps)}},
            )

        def insert_records(conn, profile, wallet_row, records, source_label):
            pass_number = records[0]["pass"]
            if pass_number == 1:
                conn.execute(
                    """
                    INSERT OR IGNORE INTO transactions(
                        id, profile_id, wallet_id, external_id, occurred_at,
                        direction, asset, amount, fee, excluded, created_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
                    """,
                    (
                        "out-1",
                        profile["id"],
                        wallet_row["id"],
                        "tx-out",
                        "2026-01-02T00:00:00Z",
                        "outbound",
                        "BTC",
                        1000,
                        0,
                        "2026-01-02T00:00:01Z",
                    ),
                )
            else:
                conn.executemany(
                    """
                    INSERT OR IGNORE INTO transactions(
                        id, profile_id, wallet_id, external_id, occurred_at,
                        direction, asset, amount, fee, excluded, created_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)
                    """,
                    [
                        (
                            "in-1",
                            profile["id"],
                            wallet_row["id"],
                            "tx-in",
                            "2026-01-01T00:00:00Z",
                            "inbound",
                            "BTC",
                            1000,
                            0,
                            "2026-01-01T00:00:01Z",
                        ),
                        (
                            "out-1",
                            profile["id"],
                            wallet_row["id"],
                            "tx-out",
                            "2026-01-02T00:00:00Z",
                            "outbound",
                            "BTC",
                            1000,
                            0,
                            "2026-01-02T00:00:01Z",
                        ),
                    ],
                )
            return {"imported": 1, "skipped": 0, "unchanged": 0}

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=insert_records,
            resolve_backend=lambda runtime_config, backend_name: {
                "name": "default",
                "kind": "esplora",
                "url": "https://example.invalid",
            },
            resolve_sync_state=resolve_sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": adapter},
        )

        with patch("kassiber.core.sync.source_overlap.raise_for_sync_source_overlap"):
            outcome = sync_wallet_from_backend(conn, {}, profile, wallet, hooks)

        self.assertEqual(
            adapter_gaps,
            [DEFAULT_DESCRIPTOR_GAP_LIMIT, NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT],
        )
        self.assertIsNone(checkpoints_seen[0])
        self.assertEqual(checkpoints_seen[1], {})
        self.assertTrue(outcome["force_full"])
        self.assertEqual(outcome["gap_limit"], NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT)
        self.assertEqual(
            outcome["negative_balance_rescan"]["original_gap_limit"],
            DEFAULT_DESCRIPTOR_GAP_LIMIT,
        )
        self.assertEqual(
            outcome["negative_balance_rescan"]["rescan_gap_limit"],
            NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT,
        )
        self.assertTrue(outcome["negative_balance_rescan"]["resolved"])
        self.assertEqual(
            outcome["negative_balance_rescan"]["initial_negative_events"][0][
                "transaction_id"
            ],
            "out-1",
        )
        self.assertEqual(
            outcome["negative_balance_rescan"]["remaining_negative_events"],
            [],
        )

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

        def fake_fetch(
            base_url,
            script_pubkey_hex,
            max_pages=None,
            timeout=30,
            proxy_url=None,
        ):
            fetch_calls.append(
                (base_url, script_pubkey_hex, max_pages, timeout, proxy_url)
            )
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

    def test_esplora_incremental_refetches_when_balance_changes(self):
        # Companion to test_esplora_checkpoint_skips_unchanged_script_pages:
        # the unchanged-script skip is rate-limit-safe, but it MUST NOT skip a
        # script whose on-chain stats changed. A new deposit changes the
        # fingerprint (funded_txo_sum / tx_count), so the incremental sync has
        # to re-fetch that script and emit the new record — otherwise the
        # summed wallet balance would only move on a full rescan. Regression
        # guard for the reported "balances only update on a full rescan" bug.
        target = {"address": "bc1qesplora", "script_pubkey": "0014" + "11" * 20}
        tx1 = {
            "txid": "11" * 32,
            "fee": 0,
            "vin": [],
            "vout": [{"scriptpubkey": target["script_pubkey"], "value": 12_345}],
            "status": {"block_time": 1_700_000_000},
        }
        tx2 = {
            "txid": "22" * 32,
            "fee": 0,
            "vin": [],
            "vout": [{"scriptpubkey": target["script_pubkey"], "value": 50_000}],
            "status": {"block_time": 1_700_100_000},
        }
        stats_first = {
            "chain_stats": {
                "funded_txo_count": 1,
                "funded_txo_sum": 12_345,
                "spent_txo_count": 0,
                "spent_txo_sum": 0,
                "tx_count": 1,
            },
            "mempool_stats": {"tx_count": 0},
        }
        stats_second = {
            "chain_stats": {
                "funded_txo_count": 2,
                "funded_txo_sum": 62_345,
                "spent_txo_count": 0,
                "spent_txo_sum": 0,
                "tx_count": 2,
            },
            "mempool_stats": {"tx_count": 0},
        }
        backend = {
            "name": "esplora",
            "kind": "esplora",
            "url": "https://esplora.example",
        }
        wallet = {
            "id": "wallet-1",
            "config_json": '{"birthday": "2024-01-01T00:00:00Z"}',
        }

        def make_state(checkpoint=None):
            return WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=[target],
                tracked_scripts={target["script_pubkey"]: target},
                history_cache={},
                **({"checkpoint": checkpoint} if checkpoint else {}),
            )

        with patch(
            "kassiber.core.sync_backends.esplora_scripthash_stats",
            side_effect=[stats_first, stats_second],
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_transactions",
            side_effect=[[tx1], [tx1, tx2]],
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_utxos",
            return_value=[],
        ):
            records1, meta1 = esplora_sync_adapter(backend, wallet, make_state())
            records2, meta2 = esplora_sync_adapter(
                backend, wallet, make_state(meta1["freshness_checkpoint"])
            )

        # First sync establishes the single deposit.
        self.assertEqual({record["txid"] for record in records1}, {"11" * 32})
        # Incremental sync sees the changed fingerprint, re-fetches, and emits
        # the new deposit so the summed balance grows from 12_345 to 62_345.
        self.assertEqual(meta2["scripts_unchanged"], 0)
        self.assertEqual(meta2["scripts_changed"], 1)
        self.assertEqual(
            {record["txid"] for record in records2}, {"11" * 32, "22" * 32}
        )

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
            proxy_url=None,
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
        progress = []

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
            token = sync_progress_emitter.set(lambda payload: progress.append(dict(payload)))
            try:
                targets = scan_descriptor_targets(
                    plan,
                    target_used_batch=target_used_batch,
                    scan_batch_size=1,
                    highest_used={"0": 2},
                )
            finally:
                sync_progress_emitter.reset(token)

        self.assertEqual([target["address_index"] for target in targets], [0, 1, 2, 3, 4])
        self.assertEqual(checked, [3, 4])
        self.assertEqual(progress[-1]["processed"], 2)
        self.assertNotIn("total", progress[-1])
        self.assertEqual(progress[-1]["retained_targets"], 5)
        self.assertEqual(progress[-1]["unused_streak"], 2)
        self.assertEqual(progress[-1]["gap_limit"], 2)

    def test_first_sync_scans_full_gap_depth_following_used_addresses(self):
        # A first sync (no stored checkpoint, highest_used=None) must walk the
        # FULL active range, not a fixed shallow window: every used address
        # resets the trailing-gap counter so discovery extends as deep as
        # activity goes, stopping only after gap_limit consecutive unused. This
        # pins the "first Refresh reaches the entire gap limit" trust property
        # (the original Issue #1 complaint) so it cannot silently regress.
        class FakeDescriptor:
            is_wildcard = True

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=3,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(0, "receive", FakeDescriptor()),),
        )
        used_indices = {0, 3, 6}

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
            return [target["address_index"] in used_indices for target in targets]

        with patch(
            "kassiber.core.sync_backends.derive_descriptor_targets",
            side_effect=fake_derive,
        ):
            targets = scan_descriptor_targets(
                plan,
                target_used_batch=target_used_batch,
                scan_batch_size=1,
                highest_used=None,  # first sync: derive from index 0
            )

        scanned = [target["address_index"] for target in targets]
        # Used at 0/3/6 keep the walk alive past a naive shallow scan; it stops
        # only after gap_limit(3) trailing unused (7, 8, 9), reaching index 9.
        self.assertEqual(scanned, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9])

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
        wallet = {
            "id": "wallet-1",
            "config_json": '{"birthday": "2024-01-01T00:00:00Z"}',
        }

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
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
                self.assertEqual(timeout, 1800)
                self.assertEqual(
                    params,
                    [
                        [
                            {
                                "desc": "addr(bc1qcore)#abcd",
                                "timestamp": iso_to_unix("2024-01-01T00:00:00Z"),
                                "label": "kassiber:wallet-1",
                            }
                        ]
                    ],
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

    def test_bitcoinrpc_descriptor_discovery_is_read_only(self):
        class FakeDescriptor:
            is_wildcard = True

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=2,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(6, "p2tr receive", FakeDescriptor()),),
        )
        target = {"address": "bc1qcore", "script_pubkey": "0014core"}

        with patch(
            "kassiber.core.sync_backends.bitcoinrpc_call",
            side_effect=AssertionError("discovery must not call Core"),
        ), patch(
            "kassiber.core.sync_backends._bitcoinrpc_descriptor_targets_for_checkpoint",
            return_value=[target],
        ) as derive_targets:
            discovery = discover_descriptor_targets(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                plan,
                "bitcoinrpc",
                checkpoint={"highest_used": {"6": 4}},
            )

        self.assertEqual(discovery, {"targets": [target], "history_cache": {}})
        derive_targets.assert_called_once_with(
            plan,
            {"highest_used": {"6": 4}},
        )

    def test_bitcoinrpc_descriptor_sync_imports_ranged_descriptors_and_checkpoint(self):
        class FakeDescriptor:
            is_wildcard = True

            def __init__(self, raw):
                self.raw = raw

            def to_string(self):
                return self.raw

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=3,
            descriptor_fingerprint="fp",
            branches=(
                DescriptorBranch(6, "p2tr receive", FakeDescriptor("tr(xpub/0/*)")),
                DescriptorBranch(7, "p2tr change", FakeDescriptor("tr(xpub/1/*)")),
            ),
        )
        targets = [
            {
                "address": "bc1preceive2",
                "script_pubkey": "5120" + "11" * 32,
                "branch_index": 6,
                "branch_label": "p2tr receive",
                "address_index": 2,
            },
            {
                "address": "bc1pchange0",
                "script_pubkey": "5120" + "22" * 32,
                "branch_index": 7,
                "branch_label": "p2tr change",
                "address_index": 0,
            },
        ]
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=plan,
            policy_asset_id="",
            targets=targets,
            tracked_scripts={target["script_pubkey"]: target for target in targets},
            history_cache={},
        )
        wallet = {
            "id": "wallet-1",
            "config_json": '{"birthday": "2024-01-01T00:00:00Z"}',
        }
        calls = []

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend
            calls.append((method, params, wallet_name, timeout))
            if method == "listwallets":
                return ["kassiber-wallet-1"]
            if method == "getdescriptorinfo":
                return {"descriptor": f"{params[0]}#core"}
            if method == "importdescriptors":
                descriptors = params[0]
                self.assertEqual(wallet_name, "kassiber-wallet-1")
                self.assertEqual(timeout, 1800)
                self.assertEqual(
                    descriptors,
                    [
                        {
                            "desc": "tr(xpub/0/*)#core",
                            "timestamp": iso_to_unix("2024-01-01T00:00:00Z"),
                            "range": [0, 2],
                            "internal": False,
                            "active": False,
                        },
                        {
                            "desc": "tr(xpub/1/*)#core",
                            "timestamp": iso_to_unix("2024-01-01T00:00:00Z"),
                            "range": [0, 2],
                            "internal": True,
                            "active": False,
                        },
                    ],
                )
                return [{"success": True}, {"success": True}]
            if method == "listtransactions":
                return [
                    {
                        "txid": "77" * 32,
                        "category": "receive",
                        "amount": 0.003,
                        "fee": 0,
                        "blocktime": 1_700_000_200,
                    }
                ]
            if method == "getbestblockhash":
                return "cc" * 32
            if method == "listunspent":
                self.assertEqual(params, [0, 9999999, ["bc1preceive2", "bc1pchange0"], True])
                return [
                    {
                        "txid": "88" * 32,
                        "vout": 0,
                        "address": "bc1preceive2",
                        "amount": 0.004,
                        "confirmations": 4,
                    }
                ]
            if method == "gettransaction":
                return {"blockhash": "dd" * 32, "blocktime": 1_700_000_300}
            if method == "getblockheader":
                return {"height": 123}
            raise AssertionError(f"Unexpected RPC call: {(method, params, wallet_name)!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call), patch(
            "kassiber.core.sync_backends._bitcoinrpc_descriptor_targets_for_range_ends",
            return_value=targets,
        ):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                wallet,
                sync_state,
            )

        self.assertEqual(meta["imported_descriptors"], 2)
        self.assertEqual(meta["bitcoinrpc_sync_mode"], "full_scan")
        self.assertEqual(
            meta["freshness_checkpoint"]["bitcoinrpc_descriptor_range_ends"],
            {"6": 2, "7": 2},
        )
        self.assertEqual(meta["freshness_checkpoint"]["highest_used"], {"6": 2})
        self.assertEqual(len(meta["utxos"]), 1)
        self.assertEqual(meta["utxos"][0]["branch_index"], 6)
        self.assertEqual(len(records), 1)
        self.assertNotIn("scantxoutset", [call[0] for call in calls])

    def test_bitcoinrpc_fixed_descriptor_import_omits_range(self):
        class FakeDescriptor:
            is_wildcard = False

            def to_string(self):
                return "wpkh(xpub/0/5)"

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=3,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(0, "receive", FakeDescriptor()),),
        )

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, timeout
            if method == "getdescriptorinfo":
                return {"descriptor": "wpkh(xpub/0/5)#core"}
            if method == "importdescriptors":
                self.assertEqual(wallet_name, "kassiber-wallet-1")
                self.assertEqual(
                    params,
                    [
                        [
                            {
                                "desc": "wpkh(xpub/0/5)#core",
                                "timestamp": 0,
                                "internal": False,
                                "active": False,
                            }
                        ]
                    ],
                )
                return [{"success": True}]
            raise AssertionError(f"Unexpected RPC call: {(method, params, wallet_name)!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            imported_count, range_ends = bitcoinrpc_import_ranged_descriptors(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                "kassiber-wallet-1",
                plan,
                {},
                0,
            )

        self.assertEqual(imported_count, 1)
        self.assertEqual(range_ends, {"0": 0})

    def test_bitcoinrpc_descriptor_sync_uses_listsinceblock_when_range_unchanged(self):
        class FakeDescriptor:
            is_wildcard = True

            def __init__(self, raw):
                self.raw = raw

            def to_string(self):
                return self.raw

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=3,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(6, "p2tr receive", FakeDescriptor("tr(xpub/0/*)")),),
        )
        target = {
            "address": "bc1preceive2",
            "script_pubkey": "5120" + "11" * 32,
            "branch_index": 6,
            "branch_label": "p2tr receive",
            "address_index": 2,
        }
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=plan,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
            checkpoint={
                "bitcoinrpc_last_block": "aa" * 32,
                "bitcoinrpc_descriptor_range_ends": {"6": 5},
                "highest_used": {"6": 2},
            },
        )
        calls = []

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, timeout
            calls.append((method, params, wallet_name))
            if method == "listwallets":
                return ["kassiber-wallet-1"]
            if method == "listsinceblock":
                return {
                    "transactions": [],
                    "lastblock": "bb" * 32,
                    "removed": [],
                }
            if method == "listunspent":
                return []
            raise AssertionError(f"Unexpected RPC call: {(method, params, wallet_name)!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call), patch(
            "kassiber.core.sync_backends._bitcoinrpc_descriptor_targets_for_range_ends",
            return_value=[target],
        ):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                {"id": "wallet-1", "config_json": "{}"},
                sync_state,
            )

        self.assertEqual(records, [])
        self.assertEqual(meta["imported_descriptors"], 0)
        self.assertEqual(meta["bitcoinrpc_sync_mode"], "sinceblock")
        self.assertEqual(
            [method for method, _params, _wallet in calls],
            ["listwallets", "listsinceblock", "listunspent"],
        )

    def test_bitcoinrpc_descriptor_sync_learns_highest_used_from_history(self):
        class FakeDescriptor:
            is_wildcard = True

            def __init__(self, raw):
                self.raw = raw

            def to_string(self):
                return self.raw

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=3,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(6, "p2tr receive", FakeDescriptor("tr(xpub/0/*)")),),
        )
        target = {
            "address": "bc1preceive2",
            "script_pubkey": "5120" + "11" * 32,
            "branch_index": 6,
            "branch_label": "p2tr receive",
            "address_index": 2,
        }
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=plan,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend
            if method == "listwallets":
                return ["kassiber-wallet-1"]
            if method == "getdescriptorinfo":
                return {"descriptor": f"{params[0]}#core"}
            if method == "importdescriptors":
                self.assertEqual(timeout, 1800)
                self.assertEqual(params[0][0]["range"], [0, 2])
                return [{"success": True}]
            if method == "listtransactions":
                return [
                    {
                        "txid": "55" * 32,
                        "category": "receive",
                        "address": "bc1preceive2",
                        "amount": 0.001,
                        "fee": 0,
                        "blocktime": 1_700_000_000,
                    }
                ]
            if method == "getbestblockhash":
                return "bb" * 32
            if method == "listunspent":
                return []
            raise AssertionError(f"Unexpected RPC call: {(method, params, wallet_name)!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call), patch(
            "kassiber.core.sync_backends._bitcoinrpc_descriptor_targets_for_range_ends",
            return_value=[target],
        ):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                {"id": "wallet-1", "config_json": "{}"},
                sync_state,
            )

        self.assertEqual(len(records), 1)
        self.assertEqual(meta["freshness_checkpoint"]["highest_used"], {"6": 2})

    def test_bitcoinrpc_descriptor_sync_widens_from_history_highest_used(self):
        class FakeDescriptor:
            is_wildcard = True

            def __init__(self, raw):
                self.raw = raw

            def to_string(self):
                return self.raw

        plan = DescriptorPlan(
            chain="bitcoin",
            network="bitcoin",
            gap_limit=3,
            descriptor_fingerprint="fp",
            branches=(DescriptorBranch(6, "p2tr receive", FakeDescriptor("tr(xpub/0/*)")),),
        )
        target = {
            "address": "bc1preceive2",
            "script_pubkey": "5120" + "11" * 32,
            "branch_index": 6,
            "branch_label": "p2tr receive",
            "address_index": 2,
        }
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=plan,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
            checkpoint={
                "bitcoinrpc_last_block": "aa" * 32,
                "bitcoinrpc_descriptor_range_ends": {"6": 2},
                "highest_used": {"6": 2},
            },
        )

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend
            if method == "listwallets":
                return ["kassiber-wallet-1"]
            if method == "getdescriptorinfo":
                return {"descriptor": f"{params[0]}#core"}
            if method == "importdescriptors":
                self.assertEqual(wallet_name, "kassiber-wallet-1")
                self.assertEqual(timeout, 1800)
                self.assertEqual(params[0][0]["range"], [0, 5])
                return [{"success": True}]
            if method == "listtransactions":
                return []
            if method == "getbestblockhash":
                return "bb" * 32
            if method == "listunspent":
                return []
            raise AssertionError(f"Unexpected RPC call: {(method, params, wallet_name)!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call), patch(
            "kassiber.core.sync_backends._bitcoinrpc_descriptor_targets_for_range_ends",
            return_value=[target],
        ):
            records, meta = bitcoinrpc_sync_adapter(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                {"id": "wallet-1", "config_json": "{}"},
                sync_state,
            )

        self.assertEqual(records, [])
        self.assertEqual(meta["imported_descriptors"], 1)
        self.assertEqual(
            meta["freshness_checkpoint"]["bitcoinrpc_descriptor_range_ends"],
            {"6": 5},
        )

    def test_wallet_birthday_validation_and_unix_conversion(self):
        config = core_wallets._validated_wallet_config(
            "descriptor",
            {"source_file": "/tmp/wallet.json", "birthday": "2024-01-02"},
        )

        self.assertEqual(config["birthday"], "2024-01-02T00:00:00Z")
        self.assertEqual(iso_to_unix(config["birthday"]), 1_704_153_600)
        self.assertEqual(iso_to_unix(None), 0)
        with self.assertRaises(AppError):
            core_wallets._validated_wallet_config(
                "descriptor",
                {"source_file": "/tmp/wallet.json", "birthday": "not-a-date"},
            )

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

    def test_bitcoinrpc_multi_output_send_does_not_double_count_fee(self):
        # Bitcoin Core stamps the SAME whole-tx fee on every `send`-category
        # detail of one transaction. Summing per detail would double-count it for
        # a multi-output send; the fee must be booked exactly once.
        record = record_from_bitcoinrpc_details(
            "66" * 32,
            [
                {"category": "send", "amount": -0.5, "fee": -0.0001, "blocktime": 1_700_000_000},
                {"category": "send", "amount": -0.3, "fee": -0.0001, "blocktime": 1_700_000_000},
            ],
            "core",
        )
        self.assertEqual(record["direction"], "outbound")
        # fee booked once (0.0001), not summed to 0.0002.
        self.assertAlmostEqual(float(record["fee"]), 0.0001, places=8)
        # amount = gross_out (0.8) - fee (0.0001), not - 0.0002.
        self.assertAlmostEqual(float(record["amount"]), 0.7999, places=8)


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
        with patch("kassiber.proxy.socket.create_connection", return_value=fake):
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

    def test_connect_via_socks5_authenticates_with_userpass(self):
        fake = _FakeSocket(
            [
                b"\x05\x02",  # proxy selects username/password
                b"\x01\x00",  # RFC 1929 auth success
                b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00",
            ],
        )
        with patch("kassiber.proxy.socket.create_connection", return_value=fake):
            sock = _connect_via_socks5(
                "socks5h://alice:p%40ss@127.0.0.1:9050",
                "node.example",
                50002,
                timeout=5,
            )
        self.assertIs(sock, fake)
        self.assertFalse(fake.closed)
        sent = bytes(fake.sent)
        self.assertTrue(sent.startswith(b"\x05\x02\x00\x02"))
        self.assertIn(b"\x01\x05alice\x04p@ss", sent)

    def test_connect_via_socks5_rejects_auth_required_without_credentials(self):
        fake = _FakeSocket([b"\x05\x02"])  # proxy requires user/pass
        with patch("kassiber.proxy.socket.create_connection", return_value=fake):
            with self.assertRaisesRegex(AppError, "username/password"):
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
        with patch("kassiber.proxy.socket.create_connection", return_value=fake):
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
        with patch("kassiber.proxy.socket.create_connection", return_value=fake):
            with self.assertRaises(AppError):
                _connect_via_socks5(
                    "socks5://127.0.0.1:9050",
                    "node.example",
                    50002,
                    timeout=5,
                )
        self.assertTrue(fake.closed)

    def test_connect_backend_socket_rejects_onion_without_proxy(self):
        with patch("kassiber.core.sync_backends.socket.create_connection") as direct:
            with self.assertRaisesRegex(AppError, "Tor/SOCKS proxy"):
                _connect_backend_socket(
                    {"timeout": 5},
                    "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion",
                    50001,
                )
        direct.assert_not_called()


class _FakeHttpResponse:
    def __init__(self, body):
        self._body = body if isinstance(body, bytes) else body.encode("utf-8")

    def __enter__(self):
        return self

    def __exit__(self, *exc_info):
        return False

    def read(self):
        return self._body


def _http_error(code, retry_after=None, body=b"throttled"):
    headers = Message()
    if retry_after is not None:
        headers["Retry-After"] = str(retry_after)
    return urlerror.HTTPError(
        "https://esplora.example/x", code, "err", headers, io.BytesIO(body)
    )


class HttpRetryAndLimiterTest(unittest.TestCase):
    def _patched_urlopen(self, scripted):
        queue = list(scripted)

        def fake_urlopen(request, timeout=30):
            item = queue.pop(0)
            if isinstance(item, urlerror.HTTPError):
                raise item
            return _FakeHttpResponse(item)

        return patch.object(sb.urlrequest, "urlopen", side_effect=fake_urlopen)

    def test_http_get_json_retries_on_429_then_succeeds(self):
        sleeps = []
        scripted = [
            _http_error(429, retry_after=2),
            _http_error(429, retry_after=1),
            '{"ok": true}',
        ]
        with self._patched_urlopen(scripted):
            result = sb.http_get_json(
                "https://esplora.example/x",
                _sleeper=sleeps.append,
                _rng=random.Random(0),
                _max_attempts=3,
            )
        self.assertEqual(result, {"ok": True})
        # Retry-After header is honored verbatim for the backoff delays.
        self.assertEqual(sleeps, [2.0, 1.0])

    def test_http_get_json_passes_proxy_to_shared_opener(self):
        with patch(
            "kassiber.core.sync_backends.urlopen_with_proxy",
            return_value=_FakeHttpResponse('{"ok": true}'),
        ) as opener:
            result = sb.http_get_json(
                "https://esplora.example/x",
                proxy_url="127.0.0.1:9050",
            )
        self.assertEqual(result, {"ok": True})
        self.assertEqual(opener.call_args.kwargs["proxy_url"], "127.0.0.1:9050")
        self.assertEqual(opener.call_args.kwargs["source_label"], "backend")

    def test_http_get_text_retries_on_503_then_succeeds(self):
        sleeps = []
        scripted = [_http_error(503), _http_error(503), "tip-height-body"]
        with self._patched_urlopen(scripted):
            result = sb.http_get_text(
                "https://esplora.example/x",
                _sleeper=sleeps.append,
                _rng=random.Random(0),
                _max_attempts=3,
            )
        self.assertEqual(result, "tip-height-body")
        # No Retry-After -> exponential backoff with jitter; two sleeps occurred.
        self.assertEqual(len(sleeps), 2)
        self.assertTrue(all(delay > 0 for delay in sleeps))

    def test_retry_exhaustion_reraises_rate_limited(self):
        sleeps = []
        scripted = [_http_error(429, retry_after=1), _http_error(429, retry_after=1)]
        with self._patched_urlopen(scripted):
            with self.assertRaises(AppError) as ctx:
                sb.http_get_json(
                    "https://esplora.example/x",
                    _sleeper=sleeps.append,
                    _rng=random.Random(0),
                    _max_attempts=2,
                )
        self.assertEqual(ctx.exception.code, "rate_limited")
        self.assertTrue(ctx.exception.retryable)
        # The scheduler's outer backoff still sees the server-suggested delay.
        self.assertEqual(ctx.exception.details["retry_after_seconds"], 1)
        # Only one sleep happened before the final attempt exhausted.
        self.assertEqual(sleeps, [1.0])

    def test_503_exhaustion_reraises_rate_limited(self):
        # 503 is net-new retryable behavior (previously an immediate failure);
        # pin that exhaustion still surfaces the retryable rate_limited contract
        # so the freshness scheduler's outer backoff fires.
        sleeps = []
        scripted = [_http_error(503), _http_error(503)]
        with self._patched_urlopen(scripted):
            with self.assertRaises(AppError) as ctx:
                sb.http_get_json(
                    "https://esplora.example/x",
                    _sleeper=sleeps.append,
                    _rng=random.Random(0),
                    _max_attempts=2,
                )
        self.assertEqual(ctx.exception.code, "rate_limited")
        self.assertTrue(ctx.exception.retryable)
        self.assertIn("HTTP 503", str(ctx.exception))
        self.assertEqual(len(sleeps), 1)

    def test_huge_retry_after_is_clamped_and_deferred_without_sleeping(self):
        sleeps = []
        scripted = [_http_error(429, retry_after=86_400), '{"ok": true}']
        with self._patched_urlopen(scripted):
            with self.assertRaises(AppError) as ctx:
                sb.http_get_json(
                    "https://esplora.example/x",
                    _sleeper=sleeps.append,
                    _rng=random.Random(0),
                    _max_attempts=3,
                )
        self.assertEqual(ctx.exception.code, "rate_limited")
        # A Retry-After beyond the cumulative cap must not block the sync; it is
        # re-raised immediately so the freshness scheduler owns the long cooldown.
        self.assertEqual(sleeps, [])

    def test_non_retryable_http_error_raises_immediately(self):
        sleeps = []
        scripted = [_http_error(404, body=b"missing")]
        with self._patched_urlopen(scripted):
            with self.assertRaises(AppError) as ctx:
                sb.http_get_json(
                    "https://esplora.example/x",
                    _sleeper=sleeps.append,
                    _max_attempts=3,
                )
        self.assertNotEqual(ctx.exception.code, "rate_limited")
        self.assertIn("HTTP 404", str(ctx.exception))
        self.assertEqual(sleeps, [])

    def test_host_limiter_is_shared_per_host(self):
        from kassiber import http_client

        first = http_client.host_limiter("https://shared.example/a")
        again = http_client.host_limiter("https://shared.example/b?q=1")
        other = http_client.host_limiter("https://other.example/a")
        self.assertIs(first, again)
        self.assertIsNot(first, other)

    def test_http_get_json_recovers_from_real_429(self):
        # Closest analog to a throttling public backend: a real loopback server
        # that returns HTTP 429 (Retry-After) once, then a 200 JSON body. Proves
        # the genuine urlopen -> HTTPError -> retry -> success path end to end.
        import threading
        from http.server import BaseHTTPRequestHandler, HTTPServer

        state = {"hits": 0}

        class _ThrottleHandler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                state["hits"] += 1
                if state["hits"] == 1:
                    self.send_response(429)
                    self.send_header("Retry-After", "0")
                    self.end_headers()
                    self.wfile.write(b"slow down")
                    return
                body = b'{"ok": true}'
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = HTTPServer(("127.0.0.1", 0), _ThrottleHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            host, port = server.server_address
            result = sb.http_get_json(
                f"http://{host}:{port}/scripthash/abc",
                _sleeper=lambda _delay: None,
                _max_attempts=3,
            )
        finally:
            server.shutdown()
            thread.join(timeout=5)
            server.server_close()
        self.assertEqual(result, {"ok": True})
        self.assertEqual(state["hits"], 2)


class EsploraUtxoParallelTest(unittest.TestCase):
    def test_utxos_preserve_target_order_under_parallel_fetch(self):
        targets = [
            {"address": f"bc1qaddr{index}", "script_pubkey": "0014" + f"{index:02x}" * 20}
            for index in range(5)
        ]
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=targets,
            tracked_scripts={t["script_pubkey"]: t for t in targets},
            history_cache={},
        )

        def fake_utxos(base_url, script_pubkey_hex, timeout=30, proxy_url=None):
            del base_url, timeout, proxy_url
            # Encode the target's position in the UTXO value so the output order
            # is verifiable independent of which worker finishes first.
            index = next(i for i, t in enumerate(targets) if t["script_pubkey"] == script_pubkey_hex)
            return [{"txid": f"{index:02x}" * 32, "vout": 0, "value": 1000 + index, "status": {"block_height": 100 + index}}]

        with patch(
            "kassiber.core.sync_backends._esplora_tip_height", return_value=200
        ), patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_utxos",
            side_effect=fake_utxos,
        ):
            outputs = esplora_utxos_for_wallet(
                {"name": "esplora", "kind": "esplora", "url": "https://esplora.example", "batch_size": 8},
                sync_state,
            )
        self.assertEqual([o["amount_sats"] for o in outputs], [1000, 1001, 1002, 1003, 1004])
        self.assertEqual(
            [o["address"] for o in outputs],
            [t["address"] for t in targets],
        )


def _backend_sync_wallet(wallet_id, label, address):
    return {
        "id": wallet_id,
        "label": label,
        "kind": "single",
        "config_json": json.dumps({"addresses": [address]}),
    }


def _sync_state_with_target(address, script):
    target = {"address": address, "script_pubkey": script}
    return WalletSyncState(
        chain="bitcoin",
        network="bitcoin",
        descriptor_plan=None,
        policy_asset_id="",
        targets=[target],
        tracked_scripts={script: target},
        history_cache={},
    )


class HttpBackoffProgressTest(unittest.TestCase):
    def _patched_urlopen(self, scripted):
        queue = list(scripted)

        def fake_urlopen(request, timeout=30):
            item = queue.pop(0)
            if isinstance(item, urlerror.HTTPError):
                raise item
            return _FakeHttpResponse(item)

        return patch.object(sb.urlrequest, "urlopen", side_effect=fake_urlopen)

    def test_backoff_emits_rate_limited_progress_event(self):
        # The silent-backoff fix: a 429 wait must surface as a progress event so
        # the UI can show "rate limited, retrying" instead of a frozen bar.
        progress = []
        scripted = [_http_error(429, retry_after=2), '{"ok": true}']
        token = sync_progress_emitter.set(lambda payload: progress.append(dict(payload)))
        try:
            with self._patched_urlopen(scripted):
                result = sb.http_get_json(
                    "https://esplora.example/x",
                    _sleeper=lambda _delay: None,
                    _rng=random.Random(0),
                    _max_attempts=3,
                )
        finally:
            sync_progress_emitter.reset(token)
        self.assertEqual(result, {"ok": True})
        rate_limited = [item for item in progress if item.get("phase") == "rate_limited"]
        self.assertEqual(len(rate_limited), 1)
        self.assertEqual(rate_limited[0]["retry_attempt"], 1)
        self.assertEqual(rate_limited[0]["retry_max"], 2)
        self.assertEqual(rate_limited[0]["wait_seconds"], 2.0)

    def test_map_bounded_propagates_progress_context_to_workers(self):
        # Without per-worker context propagation, emit_sync_progress() inside a
        # worker would read a default (None) emitter and silently drop the event.
        progress = []
        token = sync_progress_emitter.set(lambda payload: progress.append(dict(payload)))

        def worker(item):
            emit_sync_progress({"phase": "worker", "item": item})
            return item * 10

        try:
            results = sb._map_bounded([1, 2, 3], worker, max_workers=3)
        finally:
            sync_progress_emitter.reset(token)
        self.assertEqual(results, [10, 20, 30])
        self.assertEqual(sorted(item["item"] for item in progress), [1, 2, 3])


class CrossWalletPrefetchTest(unittest.TestCase):
    def _hooks(self, *, fail_ids=()):
        def resolve_sync_state(backend, wallet):
            if wallet["id"] in fail_ids:
                raise AppError(f"discovery failed for {wallet['id']}", code="discovery")
            address = json.loads(wallet["config_json"])["addresses"][0]
            return _sync_state_with_target(address, "0014" + "11" * 20)

        return WalletSyncHooks(
            import_file=lambda *a, **k: {},
            insert_records=lambda *a, **k: {"imported": 1, "skipped": 0},
            resolve_backend=lambda runtime_config, backend_name: {
                "name": "default",
                "kind": "esplora",
                "url": "https://esplora.example",
            },
            resolve_sync_state=resolve_sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": lambda backend, wallet, sync_state: ([], {})},
        )

    def test_classify_wallet_sync_buckets(self):
        normalize = lambda values: list(values or [])
        backend = _backend_sync_wallet("w1", "A", "bc1qaddr")
        self.assertEqual(classify_wallet_sync(backend, normalize), "backend")
        empty = {"id": "w2", "label": "B", "kind": "single", "config_json": "{}"}
        self.assertEqual(classify_wallet_sync(empty, normalize), "none")
        file_wallet = {
            "id": "w3",
            "label": "C",
            "kind": "single",
            "config_json": json.dumps({"source_file": "/tmp/x.csv", "source_format": "river_csv"}),
        }
        self.assertEqual(classify_wallet_sync(file_wallet, normalize), "file")

    def test_prefetch_returns_fetch_per_wallet_and_captures_apperror(self):
        wallets = [
            _backend_sync_wallet("w-good", "Good", "bc1qgood"),
            _backend_sync_wallet("w-bad", "Bad", "bc1qbad"),
        ]
        hooks = self._hooks(fail_ids={"w-bad"})
        prefetched = prefetch_wallets_backend({}, {}, wallets, hooks)
        self.assertIsInstance(prefetched["w-good"], WalletBackendFetch)
        self.assertEqual(prefetched["w-good"].kind, "esplora")
        self.assertIsInstance(prefetched["w-bad"], AppError)
        self.assertEqual(prefetched["w-bad"].code, "discovery")

    def test_prefetch_runs_preflight_before_backend_adapter(self):
        wallets = [
            _backend_sync_wallet("w-good", "Good", "bc1qgood"),
            _backend_sync_wallet("w-bad", "Bad", "bc1qbad"),
        ]
        adapter_calls = []

        def adapter(_backend, wallet, _sync_state):
            adapter_calls.append(wallet["id"])
            return [], {}

        hooks = self._hooks()
        hooks = WalletSyncHooks(
            import_file=hooks.import_file,
            insert_records=hooks.insert_records,
            resolve_backend=hooks.resolve_backend,
            resolve_sync_state=hooks.resolve_sync_state,
            normalize_addresses=hooks.normalize_addresses,
            backend_adapters={"esplora": adapter},
        )

        def preflight(wallet, _sync_state):
            if wallet["id"] == "w-bad":
                raise AppError("overlap", code="source_overlap")

        prefetched = prefetch_wallets_backend(
            {},
            {},
            wallets,
            hooks,
            source_overlap_preflight=preflight,
        )

        self.assertIsInstance(prefetched["w-good"], WalletBackendFetch)
        self.assertIsInstance(prefetched["w-bad"], AppError)
        self.assertEqual(prefetched["w-bad"].code, "source_overlap")
        self.assertEqual(adapter_calls, ["w-good"])

    def test_sync_wallets_applies_prefetch_and_reraises_captured_error(self):
        hooks = self._hooks()
        good = _backend_sync_wallet("w-good", "Good", "bc1qgood")
        good_fetch = fetch_wallet_backend({}, {}, good, hooks)
        # A captured fetch is applied without re-running the network fetch.
        results = sync_wallets(
            None, {}, {}, [good], hooks, prefetched={"w-good": good_fetch}
        )
        self.assertEqual(results[0]["status"], "synced")
        self.assertEqual(results[0]["backend_kind"], "esplora")
        # A captured AppError surfaces when applied (under the caller's savepoint).
        bad = _backend_sync_wallet("w-bad", "Bad", "bc1qbad")
        with self.assertRaises(AppError) as ctx:
            sync_wallets(
                None, {}, {}, [bad], hooks, prefetched={"w-bad": AppError("boom", code="x")}
            )
        self.assertEqual(ctx.exception.code, "x")


if __name__ == "__main__":
    unittest.main()
