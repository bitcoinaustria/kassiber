import io
import json
import random
import sqlite3
import tempfile
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from dataclasses import replace
from email.message import Message
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from urllib import error as urlerror

from kassiber.cli import handlers as cli_handlers
from kassiber.core import sync_backends as sb
from kassiber.core import sync as core_sync
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
    compatibility_electrum_sync_adapter,
    compatibility_esplora_sync_adapter,
    compatibility_esplora_utxos_for_wallet,
    discover_bitcoinrpc_descriptor_targets,
    discover_compatibility_descriptor_targets,
    record_from_bitcoin_esplora_tx,
    record_from_bitcoinrpc_details,
    scriptpubkey_scripthash,
)
from kassiber.core import imports as core_imports
from kassiber.core.chain_observer.provenance import (
    row_has_current_authoritative_observation,
)
from kassiber.core.imports import ImportCoordinatorHooks
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.proxy import _connect_via_socks5, _read_exact, _socks5_address
from kassiber.time_utils import iso_to_unix, now_iso, timestamp_to_iso
from kassiber.wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    DescriptorBranch,
    DescriptorPlan,
    DerivedTarget,
)


def _test_tpub() -> str:
    from embit import bip32

    account = bip32.HDKey.from_seed(b"script-detection-network").derive(
        "m/84h/0h/0h"
    ).to_public()
    return account.to_base58(version=bytes.fromhex("043587cf"))


def _header_hex(timestamp):
    return ("00" * 68) + int(timestamp).to_bytes(4, "little").hex() + ("00" * 8)


class _DummySocket:
    def __init__(self):
        self.sent = []

    def sendall(self, payload):
        self.sent.append(payload)


class SyncBackendsTest(unittest.TestCase):
    def test_script_type_detection_infers_test_network_from_tpub(self):
        backend = {
            "name": "test-esplora",
            "kind": "esplora",
            "chain": "bitcoin",
            "network": "test",
            "url": "https://example.invalid",
        }
        with patch.object(
            sb,
            "_probe_scripts_have_history",
            return_value=[False] * len(sb.SCRIPT_TYPE_BRANCH_BASE),
        ) as probe:
            detected = sb.detect_active_script_types(backend, _test_tpub())

        self.assertEqual(len(detected), len(sb.SCRIPT_TYPE_BRANCH_BASE))
        script_pubkeys = probe.call_args.args[2]
        self.assertEqual(len(script_pubkeys), len(sb.SCRIPT_TYPE_BRANCH_BASE))

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

    def test_sync_wallet_from_backend_applies_backend_retractions(self):
        wallet = {"id": "wallet-1", "label": "Cold", "config_json": "{}"}
        profile = {"id": "profile-1", "workspace_id": "workspace-1"}
        target = {"address": "bc1qwatch", "script_pubkey": "0014watch"}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="regtest",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        retracted = []
        inserted_after_retract = []

        def retract_records(conn, profile, wallet, external_ids, source_label):
            del conn, profile, wallet
            retracted.append((source_label, list(external_ids)))
            return {
                "retracted": 1,
                "journal_invalidated": True,
                "retracted_records": [{"transaction_id": "stale"}],
            }

        def insert_records(conn, profile, wallet, records, source_label, **kwargs):
            del conn, profile, wallet
            inserted_after_retract.append(
                (source_label, bool(retracted), list(records), kwargs)
            )
            return {"imported": 0, "skipped": 0, "journal_invalidated": False}

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=insert_records,
            retract_records=retract_records,
            resolve_backend=lambda runtime_config, backend_name: {},
            resolve_sync_state=lambda backend, wallet: sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={},
        )
        fetch = WalletBackendFetch(
            backend={
                "name": "core-regtest",
                "kind": "bitcoinrpc",
                "url": "http://127.0.0.1:18443",
            },
            sync_state=sync_state,
            normalized_records=[],
            adapter_meta={"bitcoinrpc_retracted_txids": ["AA" * 32, "aa" * 32]},
            kind="bitcoinrpc",
            started=0.0,
            force_full=False,
            authoritative_chain_observer=True,
        )

        outcome = sync_wallet_from_backend(
            None,
            {},
            profile,
            wallet,
            hooks,
            prefetched=fetch,
        )

        self.assertEqual(retracted, [("backend:core-regtest", ["aa" * 32])])
        self.assertEqual(inserted_after_retract[0][0], "backend:core-regtest")
        self.assertTrue(inserted_after_retract[0][1])
        self.assertEqual(
            inserted_after_retract[0][3],
            {"authoritative_chain_observer": True},
        )
        self.assertEqual(outcome["retracted"], 1)
        self.assertTrue(outcome["journal_invalidated"])
        self.assertEqual(outcome["bitcoinrpc_retracted_txids"], ["aa" * 32])

    def test_bitcoinrpc_apply_persists_closed_observer_provenance(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-core-provenance-") as tmp:
            conn = open_db(Path(tmp) / "data")
            self.addCleanup(conn.close)
            timestamp = now_iso()
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)",
                (timestamp,),
            )
            conn.execute(
                """
                INSERT INTO profiles(
                    id, workspace_id, label, fiat_currency, tax_country,
                    tax_long_term_days, gains_algorithm, created_at
                ) VALUES('profile', 'ws', 'Profile', 'EUR', 'generic', 365, 'FIFO', ?)
                """,
                (timestamp,),
            )
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json, created_at
                ) VALUES('wallet', 'ws', 'profile', 'Core', 'descriptor', '{}', ?)
                """,
                (timestamp,),
            )
            profile = conn.execute(
                "SELECT * FROM profiles WHERE id = 'profile'"
            ).fetchone()
            wallet = conn.execute(
                "SELECT * FROM wallets WHERE id = 'wallet'"
            ).fetchone()
            txid = "ab" * 32
            record = {
                "txid": txid,
                "occurred_at": "2026-01-02T00:00:00Z",
                "confirmed_at": "2026-01-02T00:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": "1",
                "fee": "0.0001",
                "raw_json": json.dumps(
                    {
                        "txid": txid,
                        "chain": "bitcoin",
                        "network": "regtest",
                        "observer": "bitcoinrpc",
                        "vin": [],
                        "vout": [],
                    },
                    sort_keys=True,
                ),
            }
            sync_state = WalletSyncState(
                chain="bitcoin",
                network="regtest",
                descriptor_plan=None,
                policy_asset_id="",
                targets=(),
                tracked_scripts={},
                history_cache={},
            )
            hooks = WalletSyncHooks(
                import_file=lambda *_args: {},
                insert_records=lambda db, scoped_profile, scoped_wallet, records, source, **kwargs: (
                    cli_handlers._insert_records_for_sync(
                        db,
                        scoped_profile,
                        scoped_wallet,
                        records,
                        source,
                        commit=False,
                        **kwargs,
                    )
                ),
                resolve_backend=lambda *_args: {},
                resolve_sync_state=lambda *_args: sync_state,
                normalize_addresses=lambda values: list(values or []),
                backend_adapters={},
            )
            fetch = WalletBackendFetch(
                backend={
                    "name": "core",
                    "kind": "bitcoinrpc",
                    "url": "http://127.0.0.1:18443",
                },
                sync_state=sync_state,
                normalized_records=(record,),
                adapter_meta={},
                kind="bitcoinrpc",
                started=0.0,
                force_full=False,
                authoritative_chain_observer=True,
            )

            with patch.object(
                core_sync.source_overlap,
                "filter_sync_state_for_canonical_owner",
                side_effect=lambda _conn, _profile, _wallet, state: state,
            ):
                sync_wallet_from_backend(
                    conn,
                    {},
                    profile,
                    wallet,
                    hooks,
                    prefetched=fetch,
                )

            observed = conn.execute(
                """
                SELECT
                    tx.*,
                    proof.authority_version AS observation_authority_version,
                    proof.graph_hash AS observation_graph_hash,
                    proof.quantity_hash AS observation_quantity_hash,
                    proof.fee_attribution AS observation_fee_attribution
                FROM transactions tx
                LEFT JOIN chain_observation_provenance proof
                  ON proof.transaction_id = tx.id
                WHERE tx.external_id = ?
                """,
                (txid,),
            ).fetchone()
            self.assertIsNotNone(observed)
            self.assertTrue(row_has_current_authoritative_observation(observed))
            proof = conn.execute(
                "SELECT observer_kinds_json FROM chain_observation_provenance"
            ).fetchone()
            self.assertEqual(json.loads(proof["observer_kinds_json"]), ["bitcoinrpc"])

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

        def insert_records(
            conn,
            profile,
            wallet_row,
            records,
            source_label,
            *,
            authoritative_chain_observer=False,
        ):
            self.assertTrue(authoritative_chain_observer)
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
            records, meta = compatibility_esplora_sync_adapter(
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

    def test_esplora_sync_adapter_forwards_backend_auth_to_every_read(self):
        target = {"address": "bc1qauth", "script_pubkey": "0014" + "22" * 20}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        auth = {"Authorization": "Bearer observer-secret"}
        with patch(
            "kassiber.core.sync_backends.esplora_scripthash_stats",
            return_value={"chain_stats": {"tx_count": 0}, "mempool_stats": {"tx_count": 0}},
        ) as stats, patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_transactions",
            return_value=[],
        ) as history, patch(
            "kassiber.core.sync_backends._esplora_tip_height",
            return_value=100,
        ) as tip, patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_utxos",
            return_value=[],
        ) as utxos:
            compatibility_esplora_sync_adapter(
                {
                    "name": "authenticated-esplora",
                    "kind": "esplora",
                    "url": "https://esplora.example",
                    "token": "observer-secret",
                },
                {"id": "wallet-1"},
                sync_state,
            )

        self.assertEqual(stats.call_args.kwargs["headers"], auth)
        self.assertEqual(history.call_args.kwargs["headers"], auth)
        self.assertEqual(tip.call_args.kwargs["headers"], auth)
        self.assertEqual(utxos.call_args.kwargs["headers"], auth)

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
            headers=None,
            proxy_url=None,
        ):
            del headers
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
            records, meta = compatibility_esplora_sync_adapter(
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
            records, second_meta = compatibility_esplora_sync_adapter(
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
            records1, meta1 = compatibility_esplora_sync_adapter(backend, wallet, make_state())
            records2, meta2 = compatibility_esplora_sync_adapter(
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
            records, meta = compatibility_electrum_sync_adapter(
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

    def test_electrum_records_backfill_prevouts_for_local_graph_consumers(self):
        target = {"address": "bc1qchange", "script_pubkey": "0014deadbeef"}
        txid = "33" * 32
        prev_txid = "22" * 32
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
            checkpoint={
                "electrum_scripthash_statuses": {scripthash: "status-1"},
            },
        )
        raw_map = {
            "current-raw": {
                "txid": txid,
                "vin": [{"txid": prev_txid, "vout": 0, "sequence": 0xFFFFFFFD}],
                "vout": [
                    {"n": 0, "script_hex": "0014" + "aa" * 20, "value_sats": 70_000},
                    {"n": 1, "script_hex": target["script_pubkey"], "value_sats": 29_000},
                ],
                "total_output_sats": 99_000,
            },
            "prev-raw": {
                "txid": prev_txid,
                "vin": [],
                "vout": [{"n": 0, "script_hex": target["script_pubkey"], "value": 0.001}],
            },
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
                    elif key == ("blockchain.scripthash.get_history", (scripthash,)):
                        responses.append([{"tx_hash": txid, "height": 123}])
                    elif key == ("blockchain.transaction.get", (txid,)):
                        responses.append("current-raw")
                    elif key == ("blockchain.transaction.get", (prev_txid,)):
                        responses.append("prev-raw")
                    elif key == ("blockchain.block.header", (123,)):
                        responses.append(_header_hex(1_700_000_000))
                    else:
                        raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_raw_transaction",
            side_effect=lambda raw_hex: raw_map[raw_hex],
        ):
            records, meta = sb.compatibility_electrum_records_for_wallet(
                {"name": "fulcrum", "kind": "electrum", "url": "ssl://electrum.example:50002"},
                sync_state,
            )

        self.assertEqual(meta["scripts_changed"], 1)
        self.assertEqual(meta["freshness_checkpoint"]["electrum_stored_graph_version"], 1)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["direction"], "outbound")
        raw = json.loads(records[0]["raw_json"])
        self.assertEqual(
            raw["vin"][0]["prevout"],
            {"scriptpubkey": target["script_pubkey"], "value": 100_000},
        )
        self.assertEqual(raw["vout"][0]["scriptpubkey"], "0014" + "aa" * 20)
        self.assertEqual(raw["vout"][0]["value"], 70_000)
        self.assertEqual(raw["vout"][1]["scriptpubkey"], target["script_pubkey"])
        self.assertEqual(raw["vout"][1]["value"], 29_000)
        self.assertEqual(raw["vout"][0]["script_hex"], "0014" + "aa" * 20)
        self.assertEqual(
            raw["_kassiber_electrum_graph"],
            {"kind": "bitcoin_electrum", "version": 1},
        )

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
            "kassiber.core.sync_backends.compatibility_electrum_utxos_for_wallet",
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
            records, meta = compatibility_electrum_sync_adapter(
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
            records, second_meta = compatibility_electrum_sync_adapter(
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

    def test_electrum_changed_history_fetches_only_new_transaction_graphs(self):
        target = {"address": "bc1qe1", "script_pubkey": "0014deadbeef"}
        old_txid = "22" * 32
        new_txid = "33" * 32
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        old_history = {"tx_hash": old_txid, "height": 123}
        new_history = {"tx_hash": new_txid, "height": 124}
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
                        responses.append("status-2")
                    elif key == ("blockchain.scripthash.get_history", (scripthash,)):
                        responses.append([old_history, new_history])
                    elif key == ("blockchain.transaction.get", (new_txid,)):
                        responses.append("new-raw")
                    elif key == ("blockchain.block.header", (124,)):
                        responses.append(_header_hex(1_700_000_100))
                    else:
                        raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses

        old_graph = {
            "txid": old_txid,
            "_kassiber_electrum_graph": {
                "kind": "bitcoin_electrum",
                "version": 1,
            },
            "vin": [],
            "vout": [
                {
                    "n": 0,
                    "script_hex": target["script_pubkey"],
                    "scriptpubkey": target["script_pubkey"],
                    "value_sats": 12_345,
                    "value": 12_345,
                }
            ],
            "total_output_sats": 12_345,
        }
        new_graph = {
            "txid": new_txid,
            "vin": [],
            "vout": [{"n": 0, "script_hex": target["script_pubkey"], "value_sats": 50_000}],
            "total_output_sats": 50_000,
        }
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={old_txid: old_graph},
            checkpoint={
                "electrum_stored_graph_version": 1,
                "electrum_history_entries": {scripthash: {old_txid: old_history}},
                "electrum_scripthash_statuses": {scripthash: "status-1"},
                "electrum_headers": {"123": 1_700_000_000},
            },
        )

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_raw_transaction",
            return_value=new_graph,
        ):
            records, meta = sb.compatibility_electrum_records_for_wallet(
                {"name": "fulcrum", "kind": "electrum", "url": "tcp://electrum.example:50001"},
                sync_state,
            )

        self.assertEqual([record["txid"] for record in records], [new_txid])
        self.assertEqual(meta["transactions_fetched"], 1)
        self.assertIn(("blockchain.transaction.get", (new_txid,)), calls)
        self.assertNotIn(("blockchain.transaction.get", (old_txid,)), calls)
        self.assertEqual(
            meta["freshness_checkpoint"]["electrum_history_entries"][scripthash],
            {old_txid: old_history, new_txid: new_history},
        )

    def test_electrum_rejects_unproven_stored_transaction_graph(self):
        target = {"address": "bc1qe1", "script_pubkey": "0014deadbeef"}
        txid = "66" * 32
        scripthash = scriptpubkey_scripthash(target["script_pubkey"])
        raw_graph = {
            "txid": txid,
            "vin": [],
            "vout": [{"script_hex": target["script_pubkey"], "value_sats": 12_345}],
            "total_output_sats": 12_345,
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
                        responses.append("status-2")
                    elif key == ("blockchain.scripthash.get_history", (scripthash,)):
                        responses.append([{"tx_hash": txid, "height": 123}])
                    elif key == ("blockchain.transaction.get", (txid,)):
                        responses.append("current-raw")
                    elif key == ("blockchain.block.header", (123,)):
                        responses.append(_header_hex(1_700_000_000))
                    else:
                        raise AssertionError(f"Unexpected Electrum call: {key!r}")
                return responses

        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={txid: {}},
            checkpoint={
                "electrum_stored_graph_version": 1,
                "electrum_scripthash_statuses": {scripthash: "status-1"},
            },
        )

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient), patch(
            "kassiber.core.sync_backends.decode_raw_transaction",
            return_value=raw_graph,
        ):
            records, meta = sb.compatibility_electrum_records_for_wallet(
                {"name": "fulcrum", "kind": "electrum", "url": "tcp://electrum.example:50001"},
                sync_state,
            )

        self.assertEqual(len(records), 1)
        self.assertEqual(meta["transactions_fetched"], 1)
        self.assertIn(("blockchain.transaction.get", (txid,)), calls)

    def test_electrum_pool_reuses_one_connection_for_wallet_fetches(self):
        target = {"address": "bc1qe1", "script_pubkey": "0014deadbeef"}
        state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        counts = {"init": 0, "enter": 0, "exit": 0, "batch": 0}

        class FakeElectrumClient:
            def __init__(self, _backend):
                counts["init"] += 1

            def __enter__(self):
                counts["enter"] += 1
                return self

            def __exit__(self, exc_type, exc, tb):
                counts["exit"] += 1
                return False

            def batch_call(self, requests):
                counts["batch"] += 1
                return [None for _request in requests]

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool():
                sb.compatibility_electrum_records_for_wallet(backend, state)
                sb.compatibility_electrum_records_for_wallet(backend, state)

        self.assertEqual(counts, {"init": 1, "enter": 1, "exit": 1, "batch": 2})

    def test_electrum_pool_coalesces_concurrent_wallet_batches(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        batch_sizes = []

        class FakeElectrumClient:
            def __init__(self, _backend):
                pass

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                batch_sizes.append(len(requests))
                return [params[0] for _method, params in requests]

        barrier = threading.Barrier(2)

        def fetch(client, marker):
            barrier.wait()
            return client.batch_call([("example", [marker])])

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool() as pool:
                client = pool.client(backend)
                with ThreadPoolExecutor(max_workers=2) as executor:
                    futures = [
                        executor.submit(fetch, client, marker)
                        for marker in ("wallet-a", "wallet-b")
                    ]
                    results = [future.result() for future in futures]

        self.assertEqual(batch_sizes, [2])
        self.assertEqual(results, [["wallet-a"], ["wallet-b"]])

    def test_electrum_pool_caps_coalesced_wire_batches(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
            "batch_size": 1,
        }
        wire_batches = []

        class FakeElectrumClient:
            def __init__(self, _backend):
                pass

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                wire_batches.append(list(requests))
                return [params[0] for _method, params in requests]

        barrier = threading.Barrier(2)

        def fetch(client, marker):
            barrier.wait()
            return client.batch_call([("example", [marker])])

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool() as pool:
                client = pool.client(backend)
                with ThreadPoolExecutor(max_workers=2) as executor:
                    futures = [
                        executor.submit(fetch, client, marker)
                        for marker in ("wallet-a", "wallet-b")
                    ]
                    results = [future.result() for future in futures]

        self.assertEqual([len(batch) for batch in wire_batches], [1, 1])
        self.assertEqual(results, [["wallet-a"], ["wallet-b"]])

    def test_electrum_pool_isolates_error_to_its_logical_caller(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        wire_batches = []

        class FakeElectrumClient:
            def __init__(self, _backend):
                pass

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                markers = [params[0] for _method, params in requests]
                wire_batches.append(markers)
                if len(requests) > 1 or "bad" in markers:
                    raise AppError("rejected request", code="electrum_rpc_error")
                return markers

        barrier = threading.Barrier(2)

        def fetch(client, marker):
            barrier.wait()
            return client.batch_call([("example", [marker])])

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool() as pool:
                client = pool.client(backend)
                with ThreadPoolExecutor(max_workers=2) as executor:
                    good = executor.submit(fetch, client, "good")
                    bad = executor.submit(fetch, client, "bad")
                    self.assertEqual(good.result(), ["good"])
                    with self.assertRaises(AppError):
                        bad.result()

        self.assertTrue(
            any(len(batch) == 2 and set(batch) == {"good", "bad"} for batch in wire_batches)
        )
        self.assertIn(["good"], wire_batches)
        self.assertIn(["bad"], wire_batches)

    def test_electrum_pool_fails_coalesced_callers_once_on_transport_error(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        wire_batches = []

        class FakeElectrumClient:
            def __init__(self, _backend):
                pass

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                wire_batches.append([params[0] for _method, params in requests])
                raise OSError("connection closed")

        barrier = threading.Barrier(2)

        def fetch(client, marker):
            barrier.wait()
            return client.batch_call([("example", [marker])])

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool() as pool:
                client = pool.client(backend)
                with ThreadPoolExecutor(max_workers=2) as executor:
                    futures = [
                        executor.submit(fetch, client, marker)
                        for marker in ("wallet-a", "wallet-b")
                    ]
                    for future in futures:
                        with self.assertRaises(OSError):
                            future.result()

        self.assertEqual(len(wire_batches), 1)
        self.assertEqual(set(wire_batches[0]), {"wallet-a", "wallet-b"})

    def test_electrum_pool_stops_isolation_retries_after_transport_error(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        wire_batches = []

        class FakeElectrumClient:
            def __init__(self, _backend):
                pass

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                wire_batches.append([params[0] for _method, params in requests])
                if len(requests) > 1:
                    raise AppError("rejected request", code="electrum_rpc_error")
                raise OSError("connection closed")

        barrier = threading.Barrier(2)

        def fetch(client, marker):
            barrier.wait()
            return client.batch_call([("example", [marker])])

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool() as pool:
                client = pool.client(backend)
                with ThreadPoolExecutor(max_workers=2) as executor:
                    futures = [
                        executor.submit(fetch, client, marker)
                        for marker in ("wallet-a", "wallet-b")
                    ]
                    for future in futures:
                        with self.assertRaises(OSError):
                            future.result()

        self.assertEqual(len(wire_batches), 2)
        self.assertEqual(set(wire_batches[0]), {"wallet-a", "wallet-b"})
        self.assertEqual(len(wire_batches[1]), 1)

    def test_electrum_dispatcher_close_cannot_overtake_enqueue(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        real_queue_type = sb.queue.Queue
        enqueue_entered = threading.Event()
        release_enqueue = threading.Event()

        class ControlledQueue:
            def __init__(self):
                self.inner = real_queue_type()
                self.paused = False

            def get(self, *args, **kwargs):
                return self.inner.get(*args, **kwargs)

            def get_nowait(self):
                return self.inner.get_nowait()

            def put(self, item, *args, **kwargs):
                if item is not None and not self.paused:
                    self.paused = True
                    enqueue_entered.set()
                    release_enqueue.wait()
                self.inner.put(item, *args, **kwargs)

        class FakeElectrumClient:
            def __init__(self, _backend):
                pass

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

            def batch_call(self, requests):
                return [params[0] for _method, params in requests]

        result = {}

        def call(dispatcher):
            result["value"] = dispatcher.call("example", ["wallet-a"])

        with (
            patch("kassiber.core.sync_backends.queue.Queue", ControlledQueue),
            patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient),
        ):
            dispatcher = sb._ElectrumBatchDispatcher(backend)
            call_thread = threading.Thread(target=call, args=(dispatcher,), daemon=True)
            close_thread = threading.Thread(target=dispatcher.close, daemon=True)
            call_thread.start()
            self.assertTrue(enqueue_entered.wait(timeout=1))
            close_thread.start()
            release_enqueue.set()
            call_thread.join(timeout=2)
            close_thread.join(timeout=2)

        self.assertFalse(call_thread.is_alive())
        self.assertFalse(close_thread.is_alive())
        self.assertEqual(result, {"value": "wallet-a"})

    def test_electrum_dispatcher_thread_death_fails_waiting_callers(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        real_queue_type = sb.queue.Queue

        class CrashingQueue:
            """Crash the coalesce-window get, outside the per-batch guard."""

            def __init__(self):
                self.inner = real_queue_type()

            def get(self, *args, **kwargs):
                if "timeout" in kwargs or args:
                    raise RuntimeError("simulated dispatcher thread death")
                return self.inner.get()

            def get_nowait(self):
                return self.inner.get_nowait()

            def put(self, item, *args, **kwargs):
                self.inner.put(item, *args, **kwargs)

        thread_errors = []
        with (
            patch("kassiber.core.sync_backends.queue.Queue", CrashingQueue),
            patch(
                "threading.excepthook",
                side_effect=lambda args: thread_errors.append(args.exc_value),
            ),
        ):
            dispatcher = sb._ElectrumBatchDispatcher(backend)
            try:
                with self.assertRaises(AppError) as raised:
                    dispatcher.call("blockchain.scripthash.subscribe", ["aa"])
                self.assertIn("exited unexpectedly", str(raised.exception))
                dispatcher._thread.join(timeout=2)
                self.assertFalse(dispatcher._thread.is_alive())
                with self.assertRaises(AppError) as rejected:
                    dispatcher.call("blockchain.scripthash.subscribe", ["bb"])
                self.assertIn("closed", str(rejected.exception))
            finally:
                dispatcher.close()

        self.assertEqual(len(thread_errors), 1)
        self.assertIsInstance(thread_errors[0], RuntimeError)

    def test_electrum_pool_reconnects_after_transport_failure(self):
        backend = {
            "name": "fulcrum",
            "kind": "electrum",
            "url": "tcp://electrum.example:50001",
        }
        counts = {"init": 0, "exit": 0}

        class FakeElectrumClient:
            def __init__(self, _backend):
                counts["init"] += 1
                self.instance = counts["init"]

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                counts["exit"] += 1
                return False

            def batch_call(self, requests):
                if self.instance == 1:
                    raise OSError("connection closed")
                return [params[0] for _method, params in requests]

        with patch("kassiber.core.sync_backends.ElectrumClient", FakeElectrumClient):
            with sb.shared_electrum_client_pool() as pool:
                client = pool.client(backend)
                with self.assertRaises(OSError):
                    client.call("example", ["first"])
                self.assertEqual(client.call("example", ["second"]), "second")

        self.assertEqual(counts, {"init": 2, "exit": 2})

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

    def test_electrum_client_negotiates_server_version_on_connect(self):
        class HandshakeSocket(_DummySocket):
            def makefile(self, *_args, **_kwargs):
                return io.StringIO(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": ["Frigate 1.5.3", "1.6"],
                        }
                    )
                    + "\n"
                )

            def close(self):
                pass

        socket = HandshakeSocket()

        with patch(
            "kassiber.core.sync_backends._connect_backend_socket",
            return_value=socket,
        ):
            with ElectrumClient(
                {"name": "frigate", "url": "tcp://frigate.example:50001"}
            ) as client:
                self.assertEqual(client.server_version, ["Frigate 1.5.3", "1.6"])

        self.assertIsNone(client.server_version)

        self.assertEqual(len(socket.sent), 1)
        request = json.loads(socket.sent[0].decode("utf-8"))
        self.assertEqual(request["method"], "server.version")
        self.assertEqual(request["params"], ["Kassiber", "1.6"])

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
        self.assertEqual(meta["observer_route"], "bitcoin_core_rpc")
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

    def test_bitcoinrpc_records_store_verbose_graph_for_outbound_rows(self):
        target = {"address": "bc1qchange", "script_pubkey": "0014" + "ab" * 20}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        txid = "55" * 32

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, timeout
            key = (method, tuple(params or ()), wallet_name)
            if key == ("listtransactions", ("*", 1000, 0, True), "kassiber-wallet-1"):
                return [
                    {
                        "txid": txid,
                        "category": "send",
                        "amount": -0.0375,
                        "fee": -0.00001,
                        "blocktime": 1_700_000_200,
                    },
                ]
            if key == ("getbestblockhash", (), None):
                return "bb" * 32
            if key == ("gettransaction", (txid, True, True), "kassiber-wallet-1"):
                return {
                    "decoded": {
                        "txid": txid,
                        "vin": [
                            {
                                "txid": "44" * 32,
                                "vout": 2,
                                "prevout": {
                                    "value": 1.0,
                                    "scriptPubKey": {"hex": target["script_pubkey"]},
                                },
                            }
                        ],
                        "vout": [
                            {
                                "n": 0,
                                "value": 0.0375,
                                "scriptPubKey": {"hex": "0014" + "cd" * 20},
                            },
                            {
                                "n": 1,
                                "value": 0.96249,
                                "scriptPubKey": {"hex": target["script_pubkey"]},
                            },
                        ],
                    }
                }
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            records, meta = sb.bitcoinrpc_records_for_wallet(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                {"id": "wallet-1"},
                ["bc1qchange"],
                wallet_name="kassiber-wallet-1",
                imported_count=0,
                sync_state=sync_state,
            )

        self.assertEqual(meta["bitcoinrpc_last_block"], "bb" * 32)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["direction"], "outbound")
        self.assertAlmostEqual(float(records[0]["amount"]), 0.0375, places=8)
        self.assertAlmostEqual(float(records[0]["fee"]), 0.00001, places=8)
        raw = json.loads(records[0]["raw_json"])
        self.assertEqual(raw["source"], "bitcoinrpc_gettransaction")
        self.assertEqual(raw["vin"][0]["txid"], "44" * 32)
        self.assertEqual(raw["vout"][0]["scriptpubkey"], "0014" + "cd" * 20)
        self.assertEqual(raw["vout"][1]["scriptpubkey"], target["script_pubkey"])

    def test_bitcoinrpc_verbose_graph_cache_feeds_utxo_metadata(self):
        target = {"address": "bc1qchange", "script_pubkey": "0014" + "ab" * 20}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        txid = "58" * 32
        calls = []

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, timeout
            key = (method, tuple(params or ()), wallet_name)
            calls.append(key)
            if key == ("listtransactions", ("*", 1000, 0, True), "kassiber-wallet-1"):
                return [
                    {
                        "txid": txid,
                        "category": "send",
                        "amount": -0.0375,
                        "fee": -0.00001,
                        "blocktime": 1_700_000_200,
                    },
                ]
            if key == ("getbestblockhash", (), None):
                return "bb" * 32
            if key == ("gettransaction", (txid, True, True), "kassiber-wallet-1"):
                return {
                    "blockhash": "cc" * 32,
                    "blocktime": 1_700_000_200,
                    "decoded": {
                        "txid": txid,
                        "vin": [
                            {
                                "txid": "44" * 32,
                                "vout": 0,
                                "prevout": {
                                    "value": 0.03751,
                                    "scriptPubKey": {"hex": target["script_pubkey"]},
                                },
                            }
                        ],
                        "vout": [
                            {
                                "n": 0,
                                "value": 0.0375,
                                "scriptPubKey": {"hex": "0014" + "cd" * 20},
                            },
                        ],
                    },
                }
            if key == (
                "listunspent",
                (0, 9999999, ["bc1qchange"], True),
                "kassiber-wallet-1",
            ):
                return [
                    {
                        "txid": txid,
                        "vout": 1,
                        "address": "bc1qchange",
                        "amount": 0.001,
                        "confirmations": 2,
                    }
                ]
            if key == ("getblockheader", ("cc" * 32,), None):
                return {"height": 321}
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            records, meta = sb.bitcoinrpc_records_for_wallet(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                {"id": "wallet-1"},
                ["bc1qchange"],
                wallet_name="kassiber-wallet-1",
                imported_count=0,
                sync_state=sync_state,
            )
            utxos = sb.bitcoinrpc_utxos_for_wallet_name(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                "kassiber-wallet-1",
                ["bc1qchange"],
                sync_state,
                tx_cache=meta["_bitcoinrpc_verbose_tx_cache"],
            )

        self.assertEqual(len(records), 1)
        self.assertEqual(utxos[0]["block_height"], 321)
        self.assertEqual(
            [
                call
                for call in calls
                if call[0] == "gettransaction"
            ],
            [("gettransaction", (txid, True, True), "kassiber-wallet-1")],
        )

    def test_bitcoinrpc_sinceblock_immature_rows_keep_maturity_checkpoint(self):
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
                            "txid": "45" * 32,
                            "category": "immature",
                            "amount": 50,
                            "confirmations": 10,
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
        self.assertEqual(records, [])
        self.assertTrue(meta["bitcoinrpc_pending_maturity"])
        self.assertTrue(meta["freshness_checkpoint"]["bitcoinrpc_pending_maturity"])

    def test_bitcoinrpc_pending_maturity_checkpoint_forces_full_scan_until_mature(self):
        target = {"address": "bc1qcore", "script_pubkey": "0014core"}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
            checkpoint={
                "bitcoinrpc_last_block": "aa" * 32,
                "bitcoinrpc_pending_maturity": True,
            },
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
            if key == ("listtransactions", ("*", 1000, 0, True), "kassiber-wallet-1"):
                return [
                    {
                        "txid": "45" * 32,
                        "category": "generate",
                        "amount": 50,
                        "confirmations": 101,
                        "blocktime": 1_700_000_100,
                    }
                ]
            if key == ("getbestblockhash", (), None):
                return "bb" * 32
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

        self.assertNotIn("listsinceblock", calls)
        self.assertEqual(meta["bitcoinrpc_sync_mode"], "full_scan")
        self.assertFalse(meta["bitcoinrpc_pending_maturity"])
        self.assertNotIn("bitcoinrpc_pending_maturity", meta["freshness_checkpoint"])
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["txid"], "45" * 32)

    def test_compatibility_descriptor_discovery_reaches_history_beyond_local_horizon(self):
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

        with patch(
            "kassiber.core.sync_backends.derive_descriptor_targets",
            side_effect=fake_derive,
        ), patch(
            "kassiber.core.sync_backends.esplora_scripthash_has_history",
            side_effect=lambda _url, script, **_kwargs: int(script, 16)
            in used_indices,
        ) as has_history:
            discovery = discover_compatibility_descriptor_targets(
                {
                    "name": "compatibility",
                    "kind": "esplora",
                    "url": "https://esplora.example",
                    "batch_size": 1,
                    "auth_header": "Bearer discovery-secret",
                },
                plan,
                "esplora",
            )

        self.assertEqual(
            [target["address_index"] for target in discovery["targets"]],
            list(range(10)),
        )
        self.assertTrue(has_history.call_args_list)
        self.assertTrue(
            all(
                item.kwargs["headers"]
                == {"Authorization": "Bearer discovery-secret"}
                for item in has_history.call_args_list
            )
        )

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
            discovery = discover_bitcoinrpc_descriptor_targets(
                plan,
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

    def test_esplora_same_wallet_consolidation_books_only_network_fee(self):
        tracked_script = "0014watch"
        tx = {
            "txid": "45" * 32,
            "fee": 100,
            "vin": [
                {
                    "prevout": {
                        "scriptpubkey": tracked_script,
                        "value": 10_000,
                    }
                }
            ],
            "vout": [{"scriptpubkey": tracked_script, "value": 9_900}],
            "status": {"confirmed": True, "block_time": 1_700_000_000},
        }

        record = record_from_bitcoin_esplora_tx(
            tx, {tracked_script: {"address": "bc1qwatch"}}, "esplora"
        )

        self.assertEqual(record["direction"], "outbound")
        self.assertEqual(record["kind"], "fee")
        self.assertEqual(float(record["amount"]), 0.0)
        self.assertAlmostEqual(float(record["fee"]), 0.000001, places=8)

    def test_electrum_same_wallet_consolidation_books_only_network_fee(self):
        tracked_script = "0014watch"
        previous = {
            "vout": [
                {"n": 0, "script_hex": tracked_script, "value_sats": 10_000}
            ]
        }
        tx = {
            "vin": [{"txid": "44" * 32, "vout": 0}],
            "vout": [
                {"n": 0, "script_hex": tracked_script, "value_sats": 9_900}
            ],
            "total_output_sats": 9_900,
        }

        record = sb.record_from_electrum_tx(
            "46" * 32,
            tx,
            1,
            {tracked_script: {"address": "bc1qwatch"}},
            "electrum",
            lambda _txid: previous,
        )

        self.assertEqual(record["direction"], "outbound")
        self.assertEqual(record["kind"], "fee")
        self.assertEqual(float(record["amount"]), 0.0)
        self.assertAlmostEqual(float(record["fee"]), 0.000001, places=8)

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
        # a multi-output send; the fee must be booked exactly once and kept
        # separate from Core's already fee-exclusive send amounts.
        record = record_from_bitcoinrpc_details(
            "66" * 32,
            [
                {"category": "send", "amount": -0.5, "fee": -0.0001, "blocktime": 1_700_000_000},
                {"category": "send", "amount": -0.3, "fee": -0.0001, "blocktime": 1_700_000_000},
            ],
            "core",
            raw_graph={
                "vin": [
                    {
                        "prevout": {
                            "scriptpubkey": "0014" + "ef" * 20,
                            "value": 80_010_000,
                        }
                    }
                ],
                "vout": [
                    {"scriptpubkey": "0014" + "ab" * 20, "value": 50_000_000},
                    {"scriptpubkey": "0014" + "cd" * 20, "value": 30_000_000},
                ]
            },
            tracked_scripts={"0014" + "ef" * 20},
        )
        self.assertEqual(record["direction"], "outbound")
        # fee booked once (0.0001), not summed to 0.0002.
        self.assertAlmostEqual(float(record["fee"]), 0.0001, places=8)
        # With a decoded graph, the recipient amount remains 0.8 and the fee is
        # a separate ledger component.
        self.assertAlmostEqual(float(record["amount"]), 0.8, places=8)

    def test_bitcoinrpc_multi_output_send_legacy_fallback_keeps_fee_separate(self):
        record = record_from_bitcoinrpc_details(
            "66" * 32,
            [
                {"category": "send", "amount": -0.5, "fee": -0.0001, "blocktime": 1_700_000_000},
                {"category": "send", "amount": -0.3, "fee": -0.0001, "blocktime": 1_700_000_000},
            ],
            "core",
        )
        self.assertEqual(record["direction"], "outbound")
        self.assertAlmostEqual(float(record["fee"]), 0.0001, places=8)
        # Core detail amounts are recipient value; the network fee is already
        # carried separately. Subtracting it here underreports wallet/book
        # balances and breaks ownership-derived fan-out matching.
        self.assertAlmostEqual(float(record["amount"]), 0.8, places=8)

    def test_bitcoinrpc_fee_only_self_spend_keeps_zero_amount(self):
        record = record_from_bitcoinrpc_details(
            "67" * 32,
            [
                {"category": "send", "amount": -1.0, "fee": -0.0001, "blocktime": 1_700_000_000},
                {"category": "receive", "amount": 1.0, "fee": 0, "blocktime": 1_700_000_000},
            ],
            "core",
        )
        self.assertEqual(record["direction"], "outbound")
        self.assertEqual(record["kind"], "fee")
        self.assertAlmostEqual(float(record["amount"]), 0.0, places=8)
        self.assertAlmostEqual(float(record["fee"]), 0.0001, places=8)

    def test_bitcoinrpc_multi_source_graph_keeps_wallet_local_amount(self):
        record = record_from_bitcoinrpc_details(
            "66" * 32,
            [
                {"category": "send", "amount": -0.3, "fee": -0.0001, "blocktime": 1_700_000_000},
            ],
            "core",
            raw_graph={
                "vin": [
                    {
                        "prevout": {
                            "scriptpubkey": "0014" + "ab" * 20,
                            "value": 30_000_000,
                        }
                    },
                    {
                        "prevout": {
                            "scriptpubkey": "0014" + "cd" * 20,
                            "value": 70_000_000,
                        }
                    },
                ],
                "vout": [
                    {"scriptpubkey": "0014" + "ef" * 20, "value": 99_990_000},
                ],
            },
            tracked_scripts={"0014" + "ab" * 20},
        )
        self.assertEqual(record["direction"], "outbound")
        self.assertAlmostEqual(float(record["amount"]), 0.2999, places=8)

    def test_bitcoinrpc_mixed_input_graph_marks_privacy_boundary(self):
        record = record_from_bitcoinrpc_details(
            "66" * 32,
            [
                {"category": "send", "amount": -0.3, "fee": -0.0001, "blocktime": 1_700_000_000},
            ],
            "core",
            raw_graph={
                "vin": [
                    {
                        "prevout": {
                            "scriptpubkey": "0014" + "ab" * 20,
                            "value": 30_000_000,
                        }
                    },
                    {
                        "prevout": {
                            "scriptpubkey": "0014" + "cd" * 20,
                            "value": 70_000_000,
                        }
                    },
                ],
                "vout": [
                    {"scriptpubkey": "0014" + "ef" * 20, "value": 99_990_000},
                ],
            },
            tracked_scripts={"0014" + "ab" * 20},
        )
        self.assertEqual(record["privacy_boundary"], "payjoin")
        raw_json = json.loads(record["raw_json"])
        self.assertEqual(raw_json["privacy_boundary"], "payjoin")

    def test_bitcoinrpc_verbose_transaction_graph_is_esplora_shaped(self):
        graph = sb._bitcoinrpc_normalized_graph(
            "66" * 32,
            {
                "decoded": {
                    "txid": "66" * 32,
                    "vin": [{"txid": "55" * 32, "vout": 1}],
                    "vout": [
                        {
                            "n": 0,
                            "value": 0.12345678,
                            "scriptPubKey": {"hex": "0014" + "ab" * 20},
                        }
                    ],
                }
            },
        )
        self.assertEqual(graph["vin"][0]["txid"], "55" * 32)
        self.assertEqual(graph["vin"][0]["vout"], 1)
        self.assertEqual(graph["vout"][0]["scriptpubkey"], "0014" + "ab" * 20)
        self.assertEqual(graph["vout"][0]["value"], 12_345_678)

    def test_bitcoinrpc_graph_unavailable_keeps_sync_incomplete(self):
        target = {"address": "bc1qchange", "script_pubkey": "0014" + "ab" * 20}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        txid = "56" * 32

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, timeout
            key = (method, tuple(params or ()), wallet_name)
            if key == ("listtransactions", ("*", 1000, 0, True), "kassiber-wallet-1"):
                return [
                    {
                        "txid": txid,
                        "category": "send",
                        "amount": -0.3,
                        "fee": -0.00001,
                        "blocktime": 1_700_000_200,
                    },
                ]
            if key == ("getbestblockhash", (), None):
                return "bb" * 32
            if key == ("gettransaction", (txid, True, True), "kassiber-wallet-1"):
                raise AppError("temporary Core failure")
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            records, meta = sb.bitcoinrpc_records_for_wallet(
                {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                {"id": "wallet-1"},
                ["bc1qchange"],
                wallet_name="kassiber-wallet-1",
                imported_count=0,
                sync_state=sync_state,
            )

        self.assertEqual(len(records), 1)
        self.assertEqual(meta["bitcoinrpc_graph_unavailable_txids"], [txid])
        self.assertNotIn("bitcoinrpc_last_block", meta)

    def test_bitcoinrpc_missing_verbose_graph_assertion_is_not_swallowed(self):
        target = {"address": "bc1qchange", "script_pubkey": "0014" + "ab" * 20}
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        txid = "57" * 32

        def fake_bitcoinrpc_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, timeout
            key = (method, tuple(params or ()), wallet_name)
            if key == ("listtransactions", ("*", 1000, 0, True), "kassiber-wallet-1"):
                return [
                    {
                        "txid": txid,
                        "category": "send",
                        "amount": -0.3,
                        "fee": -0.00001,
                        "blocktime": 1_700_000_200,
                    },
                ]
            if key == ("getbestblockhash", (), None):
                return "bb" * 32
            raise AssertionError(f"Unexpected RPC call: {key!r}")

        with patch("kassiber.core.sync_backends.bitcoinrpc_call", side_effect=fake_bitcoinrpc_call):
            with self.assertRaises(AssertionError):
                sb.bitcoinrpc_records_for_wallet(
                    {"name": "core", "kind": "bitcoinrpc", "url": "http://core.example"},
                    {"id": "wallet-1"},
                    ["bc1qchange"],
                    wallet_name="kassiber-wallet-1",
                    imported_count=0,
                    sync_state=sync_state,
                )

    def test_bitcoinrpc_conflicted_details_are_skipped(self):
        # An RBF-replaced original stays in the wallet with negative
        # confirmations next to its confirmed replacement; booking it would
        # double-count the disposal.
        record = record_from_bitcoinrpc_details(
            "77" * 32,
            [
                {
                    "category": "send",
                    "amount": -0.2,
                    "fee": -0.00001,
                    "confirmations": -2,
                    "time": 1_700_000_000,
                    "walletconflicts": ["88" * 32],
                }
            ],
            "core",
        )
        self.assertIsNone(record)

    def test_bitcoinrpc_mature_coinbase_imports_as_deposit(self):
        record = record_from_bitcoinrpc_details(
            "99" * 32,
            [
                {
                    "category": "generate",
                    "amount": 50.0,
                    "confirmations": 120,
                    "blocktime": 1_700_000_000,
                }
            ],
            "core",
        )
        self.assertEqual(record["direction"], "inbound")
        self.assertEqual(record["kind"], "deposit")
        self.assertAlmostEqual(float(record["amount"]), 50.0, places=8)


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

    def test_http_get_helpers_forward_explicit_authorization_header(self):
        auth = {"Authorization": "Bearer observer-secret"}
        with patch(
            "kassiber.core.sync_backends.urlopen_with_proxy",
            return_value=_FakeHttpResponse('{"ok": true}'),
        ) as opener:
            sb.http_get_json("https://esplora.example/x", headers=auth)
        self.assertEqual(
            opener.call_args.args[0].get_header("Authorization"),
            "Bearer observer-secret",
        )

        with patch(
            "kassiber.core.sync_backends.urlopen_with_proxy",
            return_value=_FakeHttpResponse("123"),
        ) as opener:
            sb.http_get_text("https://esplora.example/x", headers=auth)
        self.assertEqual(
            opener.call_args.args[0].get_header("Authorization"),
            "Bearer observer-secret",
        )

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
        with_userinfo = http_client.host_limiter("https://user:pass@SHARED.example/c")
        other_port = http_client.host_limiter("https://shared.example:8443/d")
        other = http_client.host_limiter("https://other.example/a")
        self.assertIs(first, again)
        self.assertIs(first, with_userinfo)
        self.assertIs(first, other_port)
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

        def fake_utxos(
            base_url,
            script_pubkey_hex,
            timeout=30,
            headers=None,
            proxy_url=None,
        ):
            del base_url, timeout, headers, proxy_url
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
            outputs = compatibility_esplora_utxos_for_wallet(
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

    def test_prefetch_captures_non_app_error_per_wallet(self):
        wallets = [
            _backend_sync_wallet("w-good", "Good", "bc1qgood"),
            _backend_sync_wallet("w-bad", "Bad", "bc1qbad"),
        ]
        hooks = self._hooks()

        def preflight(wallet, sync_state):
            if wallet["id"] == "w-bad":
                raise RuntimeError("backend library failed")
            return sync_state

        prefetched = prefetch_wallets_backend(
            {},
            {},
            wallets,
            hooks,
            source_overlap_preflight=preflight,
        )

        self.assertIsInstance(prefetched["w-good"], WalletBackendFetch)
        self.assertIsInstance(prefetched["w-bad"], RuntimeError)

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

    def test_prefetch_uses_sync_state_returned_by_preflight(self):
        wallet = _backend_sync_wallet("w-filter", "Filtered", "bc1qfilter")
        targets = [
            {"address": "bc1qoverlap", "script_pubkey": "0014" + "11" * 20},
            {"address": "bc1qunique", "script_pubkey": "0014" + "22" * 20},
        ]
        adapter_target_counts = []

        def resolve_sync_state(_backend, _wallet):
            return WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=None,
                policy_asset_id="",
                targets=targets,
                tracked_scripts={target["script_pubkey"]: target for target in targets},
                history_cache={},
            )

        def preflight(_wallet, sync_state):
            kept = [sync_state.targets[1]]
            return WalletSyncState(
                chain=sync_state.chain,
                network=sync_state.network,
                descriptor_plan=sync_state.descriptor_plan,
                policy_asset_id=sync_state.policy_asset_id,
                targets=kept,
                tracked_scripts={kept[0]["script_pubkey"]: kept[0]},
                history_cache=sync_state.history_cache,
                checkpoint=sync_state.checkpoint,
            )

        def adapter(_backend, _wallet, sync_state):
            adapter_target_counts.append(len(sync_state.targets))
            return [], {}

        hooks = WalletSyncHooks(
            import_file=lambda *a, **k: {},
            insert_records=lambda *a, **k: {"imported": 0, "skipped": 0},
            resolve_backend=lambda runtime_config, backend_name: {
                "name": "default",
                "kind": "esplora",
                "url": "https://esplora.example",
            },
            resolve_sync_state=resolve_sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": adapter},
        )

        prefetched = prefetch_wallets_backend(
            {},
            {},
            [wallet],
            hooks,
            source_overlap_preflight=preflight,
        )

        self.assertIsInstance(prefetched["w-filter"], WalletBackendFetch)
        self.assertEqual(adapter_target_counts, [1])
        self.assertEqual(prefetched["w-filter"].sync_state.targets[0]["address"], "bc1qunique")

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


class AtomicWalletRefreshTest(unittest.TestCase):
    """Every chain projection rolls back with the coordinator savepoint."""

    STAGES = (
        core_sync.APPLY_STAGE_OBSERVER_PERSISTENCE,
        core_sync.APPLY_STAGE_RETRACTIONS,
        core_sync.APPLY_STAGE_TRANSACTION_INSERTION,
        core_sync.APPLY_STAGE_OUTPUT_INVENTORY,
        core_sync.APPLY_STAGE_DERIVATION_COVERAGE,
        core_sync.APPLY_STAGE_FRESHNESS_CHECKPOINT,
    )

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-atomic-refresh-")
        self.conn = open_db(Path(self.tmp.name) / "data")
        self.conn.executescript(
            """
            CREATE TABLE atomic_observer_state(value TEXT NOT NULL);
            CREATE TABLE atomic_projection(value TEXT NOT NULL);
            CREATE TABLE atomic_inventory(value TEXT NOT NULL);
            CREATE TABLE atomic_coverage(value TEXT NOT NULL);
            """
        )
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-atomic", "Main", _RETRACT_NOW),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "profile-atomic",
                "ws-atomic",
                "Book",
                "EUR",
                "generic",
                365,
                "FIFO",
                _RETRACT_NOW,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label,
                account_type, asset, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "account-atomic",
                "ws-atomic",
                "profile-atomic",
                "vault",
                "Vault",
                "asset",
                "BTC",
                _RETRACT_NOW,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label,
                kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wallet-atomic",
                "ws-atomic",
                "profile-atomic",
                "account-atomic",
                "Vault",
                "address",
                json.dumps({"addresses": ["bc1qatomic"], "backend": "default"}),
                _RETRACT_NOW,
            ),
        )
        self.conn.execute("INSERT INTO atomic_projection(value) VALUES('old')")
        self.conn.commit()
        self.profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile-atomic'"
        ).fetchone()
        self.wallet = self.conn.execute(
            "SELECT * FROM wallets WHERE id = 'wallet-atomic'"
        ).fetchone()
        target = {"address": "bc1qatomic", "script_pubkey": "0014" + "11" * 20}
        self.sync_state = WalletSyncState(
            chain="bitcoin",
            network="bitcoin",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target],
            tracked_scripts={target["script_pubkey"]: target},
            history_cache={},
        )
        self.fetch = WalletBackendFetch(
            backend={
                "name": "default",
                "kind": "esplora",
                "url": "https://example.invalid",
            },
            sync_state=self.sync_state,
            normalized_records=[{"external_id": "new"}],
            adapter_meta={
                "bitcoinrpc_retracted_txids": ["old"],
                "utxos": [{"outpoint": "new:0"}],
                "freshness_checkpoint": {"tip": 42},
            },
            kind="esplora",
            started=0.0,
            force_full=False,
        )

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def _snapshot(self):
        tables = (
            "atomic_observer_state",
            "atomic_projection",
            "atomic_inventory",
            "atomic_coverage",
            "freshness_source_states",
        )
        return {
            **{
                table: [tuple(row) for row in self.conn.execute(f"SELECT * FROM {table}")]
                for table in tables
            },
            "wallet": tuple(
                self.conn.execute(
                    "SELECT config_json FROM wallets WHERE id = 'wallet-atomic'"
                ).fetchone()
            ),
        }

    def _hooks(self, fail_stage, discarded=None):
        def after_stage(stage):
            if stage == fail_stage:
                raise RuntimeError(f"injected failure after {stage}")

        return WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda conn, *args: (
                conn.execute("INSERT INTO atomic_projection(value) VALUES('new')")
                and {
                    "imported": 1,
                    "skipped": 0,
                    "freshness_checkpoint": {"tip": 42},
                }
            ),
            retract_records=lambda conn, *args: (
                conn.execute("DELETE FROM atomic_projection WHERE value = 'old'")
                and {"retracted": 1, "retracted_records": []}
            ),
            resolve_backend=lambda *args: self.fetch.backend,
            resolve_sync_state=lambda *args: self.sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": lambda *args: ([], {})},
            update_output_inventory=lambda conn, *args: (
                conn.execute("INSERT INTO atomic_inventory(value) VALUES('new')")
                and {"observed": 1}
            ),
            persist_observer_update=lambda conn, *args: conn.execute(
                "INSERT INTO atomic_observer_state(value) VALUES('new')"
            ),
            update_derivation_coverage=lambda conn, *args: conn.execute(
                "INSERT INTO atomic_coverage(value) VALUES('new')"
            ),
            after_apply_stage=after_stage,
            discard_observer_update=(
                (lambda wallet: discarded.append(wallet["id"]))
                if discarded is not None
                else None
            ),
        )

    def test_every_apply_stage_failure_restores_exact_database_state(self):
        before = self._snapshot()
        for stage in self.STAGES:
            with self.subTest(stage=stage):
                discarded = []
                with patch(
                    "kassiber.core.sync.source_overlap.filter_sync_state_for_canonical_owner",
                    side_effect=lambda conn, profile, wallet, state: state,
                ):
                    with self.assertRaisesRegex(RuntimeError, stage):
                        cli_handlers._apply_wallet_sync_atomically(
                            self.conn,
                            {},
                            self.profile,
                            self.wallet,
                            self._hooks(stage, discarded),
                            prefetched={self.wallet["id"]: self.fetch},
                        )
                self.assertFalse(self.conn.in_transaction)
                self.assertEqual(self._snapshot(), before)
                self.assertEqual(discarded, ["wallet-atomic"])

    def test_success_commits_all_state_groups_together(self):
        with patch(
            "kassiber.core.sync.source_overlap.filter_sync_state_for_canonical_owner",
            side_effect=lambda conn, profile, wallet, state: state,
        ):
            results = cli_handlers._apply_wallet_sync_atomically(
                self.conn,
                {},
                self.profile,
                self.wallet,
                self._hooks(None),
                prefetched={self.wallet["id"]: self.fetch},
            )
        self.assertEqual(results[0]["status"], "synced")
        after = self._snapshot()
        self.assertIn("last_synced_at", json.loads(after["wallet"][0]))
        self.assertEqual(after["atomic_observer_state"], [("new",)])
        self.assertEqual(after["atomic_projection"], [("new",)])
        self.assertEqual(after["atomic_inventory"], [("new",)])
        self.assertEqual(after["atomic_coverage"], [("new",)])
        self.assertEqual(len(after["freshness_source_states"]), 1)

    def test_backend_fetch_finishes_before_single_wallet_savepoint(self):
        transaction_states = []

        def adapter(*args):
            transaction_states.append(("fetch", self.conn.in_transaction))
            return [], {"freshness_checkpoint": {"tip": 42}}

        def after_stage(stage):
            transaction_states.append((stage, self.conn.in_transaction))

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {
                "imported": 0,
                "skipped": 0,
                "freshness_checkpoint": {"tip": 42},
            },
            resolve_backend=lambda *args: self.fetch.backend,
            resolve_sync_state=lambda *args: self.sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": adapter},
            after_apply_stage=after_stage,
        )
        with (
            patch("kassiber.cli.handlers._wallet_sync_hooks", return_value=hooks),
            patch(
                "kassiber.core.source_overlap.filter_sync_state_for_canonical_owner",
                side_effect=lambda conn, profile, wallet, state, **_kwargs: state,
            ),
        ):
            results = cli_handlers.sync_wallet(
                self.conn,
                {},
                "ws-atomic",
                "profile-atomic",
                wallet_ref="wallet-atomic",
            )
        self.assertEqual(results[0]["status"], "synced")
        self.assertEqual(transaction_states[0], ("fetch", False))
        self.assertTrue(
            all(in_transaction for _, in_transaction in transaction_states[1:])
        )
        self.assertFalse(self.conn.in_transaction)

    def test_apply_suppresses_progress_callbacks_that_would_commit(self):
        callbacks = []

        def committing_progress(payload):
            callbacks.append(dict(payload))
            self.conn.commit()

        hooks = self._hooks(None)
        original_insert = hooks.insert_records
        hooks = replace(
            hooks,
            insert_records=lambda *args, **kwargs: (
                core_sync.emit_sync_progress({"phase": "importing"})
                or original_insert(*args, **kwargs)
            ),
        )
        token = sync_progress_emitter.set(committing_progress)
        try:
            with patch(
                "kassiber.core.sync.source_overlap.filter_sync_state_for_canonical_owner",
                side_effect=lambda conn, profile, wallet, state: state,
            ):
                results = cli_handlers._apply_wallet_sync_atomically(
                    self.conn,
                    {},
                    self.profile,
                    self.wallet,
                    hooks,
                    prefetched={self.wallet["id"]: self.fetch},
                )
        finally:
            sync_progress_emitter.reset(token)
        self.assertEqual(results[0]["status"], "synced")
        self.assertEqual(callbacks, [])
        self.assertFalse(self.conn.in_transaction)

    def test_cancellation_after_local_projection_rolls_back_everything(self):
        before = self._snapshot()
        checks = []

        def check_cancelled():
            checks.append(self.conn.in_transaction)
            if len(checks) == 2:
                raise AppError("cancelled", code="cancelled")

        with patch(
            "kassiber.core.sync.source_overlap.filter_sync_state_for_canonical_owner",
            side_effect=lambda conn, profile, wallet, state: state,
        ):
            with self.assertRaises(AppError) as ctx:
                cli_handlers._apply_wallet_sync_atomically(
                    self.conn,
                    {},
                    self.profile,
                    self.wallet,
                    self._hooks(None),
                    prefetched={self.wallet["id"]: self.fetch},
                    check_cancelled=check_cancelled,
                )
        self.assertEqual(ctx.exception.code, "cancelled")
        self.assertEqual(checks, [True, True])
        self.assertEqual(self._snapshot(), before)
        self.assertFalse(self.conn.in_transaction)

    def test_negative_balance_repair_fetch_also_finishes_before_savepoint(self):
        fetch_states = []
        gaps = []

        def resolve_sync_state(_backend, wallet):
            config = json.loads(wallet["config_json"])
            gap_limit = int(config.get("gap_limit") or DEFAULT_DESCRIPTOR_GAP_LIMIT)
            return WalletSyncState(
                chain="bitcoin",
                network="bitcoin",
                descriptor_plan=SimpleNamespace(gap_limit=gap_limit),
                policy_asset_id="",
                targets=self.sync_state.targets,
                tracked_scripts=self.sync_state.tracked_scripts,
                history_cache={},
            )

        def record(external_id, occurred_at, direction):
            return {
                "external_id": external_id,
                "occurred_at": occurred_at,
                "direction": direction,
                "asset": "BTC",
                "amount": "0.001",
                "fee": "0",
            }

        def adapter(_backend, _wallet, sync_state):
            fetch_states.append(self.conn.in_transaction)
            gaps.append(sync_state.descriptor_plan.gap_limit)
            outbound = record(
                "22" * 32,
                "2026-01-02T00:00:00Z",
                "outbound",
            )
            if len(gaps) == 1:
                return [outbound], {"freshness_checkpoint": {"pass": 1}}
            return [
                record("11" * 32, "2026-01-01T00:00:00Z", "inbound"),
                outbound,
            ], {"freshness_checkpoint": {"pass": 2}}

        hooks = WalletSyncHooks(
            import_file=lambda *args, **kwargs: {},
            insert_records=lambda *args, **kwargs: {},
            resolve_backend=lambda *args: self.fetch.backend,
            resolve_sync_state=resolve_sync_state,
            normalize_addresses=lambda values: list(values or []),
            backend_adapters={"esplora": adapter},
        )
        with patch(
            "kassiber.core.source_overlap.filter_sync_state_for_canonical_owner",
            side_effect=lambda conn, profile, wallet, state, **_kwargs: state,
        ):
            prefetched = cli_handlers._prefetch_chain_wallets(
                self.conn,
                {},
                self.profile,
                [self.wallet],
                hooks,
            )
        self.assertEqual(fetch_states, [False, False])
        self.assertEqual(
            gaps,
            [DEFAULT_DESCRIPTOR_GAP_LIMIT, NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT],
        )
        repair = prefetched[self.wallet["id"]]
        self.assertTrue(
            repair.adapter_meta["_prepared_negative_balance_rescan"]["triggered"]
        )

    def test_negative_balance_repair_failure_stays_scoped_to_wallet(self):
        first = {**self.wallet, "id": "repair-fails", "label": "Repair fails"}
        second = {**self.wallet, "id": "repair-succeeds", "label": "Repair succeeds"}
        successful_repair = replace(self.fetch, adapter_meta={})
        failure = AppError("repair failed", code="repair_failed", retryable=True)
        with (
            patch.object(
                cli_handlers,
                "_prospective_negative_balance_events",
                return_value=[{"asset": "BTC"}],
            ),
            patch.object(
                core_sync,
                "negative_balance_rescan_gap_limit",
                return_value=100,
            ),
            patch.object(
                core_sync,
                "wallet_with_temporary_gap_limit",
                side_effect=lambda wallet, _gap: wallet,
            ),
            patch.object(
                core_sync,
                "fetch_wallet_backend",
                side_effect=[failure, successful_repair],
            ),
            patch.object(core_sync, "discard_fetch_observer_updates") as discard,
        ):
            prepared = cli_handlers._prepare_negative_balance_repairs(
                self.conn,
                {},
                self.profile,
                [first, second],
                self._hooks(None),
                {first["id"]: self.fetch, second["id"]: self.fetch},
            )

        self.assertIs(prepared[first["id"]], failure)
        self.assertIsInstance(prepared[second["id"]], WalletBackendFetch)
        self.assertTrue(
            prepared[second["id"]]
            .adapter_meta["_prepared_negative_balance_rescan"]["triggered"]
        )
        self.assertEqual(discard.call_count, 2)

    def test_single_and_all_route_through_same_atomic_apply_primitive(self):
        calls = []

        def apply(conn, runtime_config, profile, wallet, hooks, **kwargs):
            calls.append(wallet["id"])
            return [{"wallet": wallet["label"], "status": "synced"}]

        with (
            patch("kassiber.cli.handlers._prefetch_chain_wallets", return_value={}),
            patch(
                "kassiber.cli.handlers._apply_wallet_sync_atomically",
                side_effect=apply,
            ),
        ):
            single = cli_handlers.sync_wallet(
                self.conn,
                {},
                "ws-atomic",
                "profile-atomic",
                wallet_ref="wallet-atomic",
            )
            all_results = cli_handlers.sync_wallet(
                self.conn,
                {},
                "ws-atomic",
                "profile-atomic",
                sync_all=True,
            )
        self.assertEqual(single[0]["status"], "synced")
        self.assertEqual(all_results[0]["status"], "synced")
        self.assertEqual(calls, ["wallet-atomic", "wallet-atomic"])

    def test_all_keeps_completed_wallet_when_later_wallet_fails(self):
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label,
                kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wallet-bad",
                "ws-atomic",
                "profile-atomic",
                "account-atomic",
                "Zulu",
                "address",
                json.dumps({"addresses": ["bc1qbad"], "backend": "default"}),
                _RETRACT_NOW,
            ),
        )
        self.conn.commit()

        def apply(conn, runtime_config, profile, wallet, hooks, **kwargs):
            if wallet["id"] == "wallet-bad":
                raise AppError("injected wallet failure", code="injected")
            return [{"wallet": wallet["label"], "status": "synced"}]

        with (
            patch("kassiber.cli.handlers._prefetch_chain_wallets", return_value={}),
            patch(
                "kassiber.cli.handlers._apply_wallet_sync_atomically",
                side_effect=apply,
            ),
        ):
            results = cli_handlers.sync_wallet(
                self.conn,
                {},
                "ws-atomic",
                "profile-atomic",
                sync_all=True,
            )
        self.assertEqual(
            [(item["wallet"], item["status"]) for item in results],
            [("Vault", "synced"), ("Zulu", "error")],
        )
        self.assertEqual(results[1]["code"], "injected")


_RETRACT_NOW = "2026-01-01T00:00:00Z"


class RetractWalletRecordsDbTest(unittest.TestCase):
    """End-to-end DB coverage for imports.retract_wallet_records.

    The sync-layer tests stub retract_records; this exercises the real DELETE +
    journal-invalidation path — the branch that removes already-booked rows when
    an authoritative backend reports RBF-replaced / orphaned txids.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-retract-")
        self.conn = open_db(Path(self.tmp.name) / "data")
        self.invalidated: list[str] = []
        self.hooks = ImportCoordinatorHooks(
            ensure_tag_row=lambda *args, **kwargs: None,
            invalidate_journals=lambda conn, profile_id: self.invalidated.append(
                profile_id
            ),
        )
        self._seed()

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def _seed(self):
        conn = self.conn
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-1", "Main", _RETRACT_NOW),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("profile-1", "ws-1", "Default", "EUR", "generic", 365, "FIFO", _RETRACT_NOW),
        )
        conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label, account_type, asset, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("acct-1", "ws-1", "profile-1", "treasury", "Treasury", "asset", "BTC", _RETRACT_NOW),
        )
        conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("wallet-a", "ws-1", "profile-1", "acct-1", "Cold", "custom", "{}", _RETRACT_NOW),
        )
        for tx_id, external_id, direction in (
            ("tx-keep", "kept-txid", "inbound"),
            ("tx-rbf-1", "rbf-original-1", "outbound"),
            ("tx-rbf-2", "rbf-original-2", "outbound"),
        ):
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                    occurred_at, direction, asset, amount, fee, fiat_currency,
                    fiat_rate, fiat_value, kind, description, counterparty, raw_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    tx_id, "ws-1", "profile-1", "wallet-a", external_id, f"fp-{tx_id}",
                    _RETRACT_NOW, direction, "BTC", 100_000_000, 0, "EUR",
                    40_000.0, None,
                    "deposit" if direction == "inbound" else "withdrawal",
                    None, None, "{}", _RETRACT_NOW,
                ),
            )
        conn.commit()

    def _external_ids(self):
        return [
            row["external_id"]
            for row in self.conn.execute(
                "SELECT external_id FROM transactions ORDER BY external_id"
            ).fetchall()
        ]

    def test_retract_deletes_matching_rows_and_invalidates_journals(self):
        result = core_imports.retract_wallet_records(
            self.conn,
            {"id": "profile-1"},
            {"id": "wallet-a", "label": "Cold"},
            # An unknown id plus a case-different duplicate prove normalization
            # and that only genuine matches are deleted.
            ["rbf-original-1", "RBF-ORIGINAL-2", "never-seen-txid"],
            "bitcoinrpc",
            self.hooks,
        )
        self.assertEqual(result["retracted"], 2)
        self.assertTrue(result["journal_invalidated"])
        self.assertEqual(len(result["retracted_records"]), 2)
        self.assertEqual(self._external_ids(), ["kept-txid"])
        self.assertEqual(self.invalidated, ["profile-1"])

    def test_retract_with_no_matches_is_a_noop(self):
        result = core_imports.retract_wallet_records(
            self.conn,
            {"id": "profile-1"},
            {"id": "wallet-a", "label": "Cold"},
            ["not-here", "also-absent"],
            "bitcoinrpc",
            self.hooks,
        )
        self.assertEqual(result["retracted"], 0)
        self.assertFalse(result["journal_invalidated"])
        self.assertEqual(
            self._external_ids(),
            ["kept-txid", "rbf-original-1", "rbf-original-2"],
        )
        self.assertEqual(self.invalidated, [])


if __name__ == "__main__":
    unittest.main()
