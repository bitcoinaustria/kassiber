import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from kassiber.core import source_overlap
from kassiber.core.sync import WalletBackendFetch, WalletSyncHooks, WalletSyncState, sync_wallet_from_backend
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.core.wallets import normalize_addresses
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.msat import btc_to_msat


ADDR_A = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
ADDR_B = "bc1q8c6fshw2dlwun7ekn9qwf37cu2rn755upcp6el"


def _script(address: str) -> str:
    from kassiber.core.address_scripts import address_to_scriptpubkey

    return address_to_scriptpubkey(address).hex()


def _seed_book(conn):
    now = "2026-01-01T00:00:00Z"
    conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)", ("ws", "Main", now))
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, last_processed_at,
            last_processed_tx_count, journal_input_version,
            last_processed_input_version, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        ("pf", "ws", "Main", "EUR", "generic", 365, "FIFO", now, 0, 0, 0, now),
    )
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "pf")
    conn.commit()
    return conn.execute("SELECT * FROM profiles WHERE id = 'pf'").fetchone()


def _wallet(conn, wallet_id, label, kind, config):
    now = "2026-01-01T00:00:00Z"
    conn.execute(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, label, kind, config_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        """,
        (wallet_id, "ws", "pf", label, kind, json.dumps(config, sort_keys=True), now),
    )
    return conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()


def _utxo(conn, wallet_id, address, txid="aa"):
    now = "2026-01-01T00:00:00Z"
    conn.execute(
        """
        INSERT INTO wallet_utxos(
            id, workspace_id, profile_id, wallet_id, backend_name, backend_kind,
            chain, network, asset, amount, txid, vout, outpoint,
            confirmation_status, confirmations, block_height, block_time,
            address, branch_label, branch_index, address_index,
            anon_history_json, first_seen_at, last_seen_at, raw_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            f"utxo-{wallet_id}-{txid}",
            "ws",
            "pf",
            wallet_id,
            "esplora",
            "esplora",
            "bitcoin",
            "mainnet",
            "BTC",
            btc_to_msat("0.01"),
            txid * 32,
            0,
            f"{txid * 32}:0",
            "confirmed",
            6,
            800000,
            now,
            address,
            "receive",
            0,
            0,
            "[]",
            now,
            now,
            "{}",
        ),
    )


def _tx(conn, tx_id, wallet_id, external_id):
    now = "2026-01-01T00:00:00Z"
    amount = btc_to_msat("0.01")
    fee = 0
    fingerprint = make_transaction_fingerprint(
        wallet_id,
        external_id,
        now,
        "inbound",
        "BTC",
        "0.01",
        "0",
    )
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, confirmed_at, direction, asset, amount, fee,
            fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
            description, counterparty, note, excluded, raw_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            tx_id,
            "ws",
            "pf",
            wallet_id,
            external_id,
            fingerprint,
            now,
            now,
            "inbound",
            "BTC",
            amount,
            fee,
            "EUR",
            50000,
            500,
            "manual",
            "deposit",
            "Deposit",
            None,
            None,
            0,
            "{}",
            now,
        ),
    )


class SourceOverlapTests(unittest.TestCase):
    def test_address_list_normalization_dedupes_script_equivalent_bech32_case(self):
        self.assertEqual(normalize_addresses([ADDR_A, ADDR_A.upper()]), [ADDR_A])

    def test_non_overlapping_address_sources_are_ready_for_overlap_detector(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-source-overlap-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                _seed_book(conn)
                _wallet(conn, "w1", "Address A", "address", {"addresses": [ADDR_A], "chain": "bitcoin", "network": "mainnet"})
                _wallet(conn, "w2", "Address B", "address", {"addresses": [ADDR_B], "chain": "bitcoin", "network": "mainnet"})
                result = source_overlap.detect_profile_source_overlaps(conn, "pf")
                self.assertEqual(result["overlaps"], [])
                self.assertFalse(result["checked"]["descriptor_global_overlap_proven"])
            finally:
                conn.close()

    def test_sync_blocks_resolved_descriptor_target_that_overlaps_address_wallet(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-source-overlap-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile = _seed_book(conn)
                _wallet(conn, "addr", "Address list", "address", {"addresses": [ADDR_A], "chain": "bitcoin", "network": "mainnet"})
                descriptor = _wallet(conn, "desc", "Descriptor", "descriptor", {"chain": "bitcoin", "network": "mainnet"})
                target = {
                    "address": ADDR_A,
                    "script_pubkey": _script(ADDR_A),
                    "chain": "bitcoin",
                    "network": "mainnet",
                    "branch_label": "receive",
                    "address_index": 0,
                }
                sync_state = WalletSyncState(
                    chain="bitcoin",
                    network="mainnet",
                    descriptor_plan=SimpleNamespace(gap_limit=20),
                    policy_asset_id="",
                    targets=[target],
                    tracked_scripts={target["script_pubkey"]: target},
                    history_cache={},
                )
                fetch = WalletBackendFetch(
                    backend={"name": "default", "kind": "esplora", "url": "https://example.invalid"},
                    sync_state=sync_state,
                    normalized_records=[{"id": "would-insert"}],
                    adapter_meta={},
                    kind="esplora",
                    started=0,
                    force_full=False,
                )
                inserted = []
                hooks = WalletSyncHooks(
                    import_file=lambda *args: {},
                    insert_records=lambda *args: inserted.append(True) or {},
                    resolve_backend=lambda *args: {},
                    resolve_sync_state=lambda *args: sync_state,
                    normalize_addresses=normalize_addresses,
                    backend_adapters={},
                )
                with self.assertRaises(AppError) as raised:
                    sync_wallet_from_backend(conn, {}, profile, descriptor, hooks, prefetched=fetch)
                self.assertEqual(raised.exception.code, "source_overlap")
                self.assertEqual(inserted, [])
                self.assertNotIn(_script(ADDR_A), json.dumps(raised.exception.details))
            finally:
                conn.close()

    def test_non_overlapping_sync_target_inserts_normally(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-source-overlap-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile = _seed_book(conn)
                _wallet(conn, "addr", "Address list", "address", {"addresses": [ADDR_A], "chain": "bitcoin", "network": "mainnet"})
                descriptor = _wallet(conn, "desc", "Descriptor", "descriptor", {"chain": "bitcoin", "network": "mainnet"})
                target = {
                    "address": ADDR_B,
                    "script_pubkey": _script(ADDR_B),
                    "chain": "bitcoin",
                    "network": "mainnet",
                    "branch_label": "receive",
                    "address_index": 0,
                }
                sync_state = WalletSyncState(
                    chain="bitcoin",
                    network="mainnet",
                    descriptor_plan=SimpleNamespace(gap_limit=20),
                    policy_asset_id="",
                    targets=[target],
                    tracked_scripts={target["script_pubkey"]: target},
                    history_cache={},
                )
                fetch = WalletBackendFetch(
                    backend={"name": "default", "kind": "esplora", "url": "https://example.invalid"},
                    sync_state=sync_state,
                    normalized_records=[],
                    adapter_meta={},
                    kind="esplora",
                    started=0,
                    force_full=False,
                )
                inserted = []
                hooks = WalletSyncHooks(
                    import_file=lambda *args: {},
                    insert_records=lambda *args: inserted.append(True) or {"imported": 0},
                    resolve_backend=lambda *args: {},
                    resolve_sync_state=lambda *args: sync_state,
                    normalize_addresses=normalize_addresses,
                    backend_adapters={},
                )
                outcome = sync_wallet_from_backend(conn, {}, profile, descriptor, hooks, prefetched=fetch)
                self.assertEqual(inserted, [True])
                self.assertEqual(outcome["sync_mode"], "descriptor")
            finally:
                conn.close()

    def test_bounded_descriptor_targets_do_not_claim_future_address_list_overlap(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-source-overlap-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile = _seed_book(conn)
                _wallet(conn, "addr", "Address list", "address", {"addresses": [ADDR_B], "chain": "bitcoin", "network": "mainnet"})
                descriptor = _wallet(conn, "desc", "Descriptor", "descriptor", {"chain": "bitcoin", "network": "mainnet"})
                target = {
                    "address": ADDR_A,
                    "script_pubkey": _script(ADDR_A),
                    "chain": "bitcoin",
                    "network": "mainnet",
                    "branch_label": "receive",
                    "address_index": 0,
                }
                sync_state = WalletSyncState(
                    chain="bitcoin",
                    network="mainnet",
                    descriptor_plan=SimpleNamespace(gap_limit=20),
                    policy_asset_id="",
                    targets=[target],
                    tracked_scripts={target["script_pubkey"]: target},
                    history_cache={},
                )
                result = source_overlap.detect_profile_source_overlaps(
                    conn,
                    "pf",
                    candidate_scripts=source_overlap.scripts_from_sync_state(profile, descriptor, sync_state),
                )
                self.assertEqual(result["overlaps"], [])
                self.assertFalse(result["checked"]["descriptor_global_overlap_proven"])
            finally:
                conn.close()

    def test_report_blocker_ignores_deprecated_overlap_until_it_has_active_transactions(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-source-overlap-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                _seed_book(conn)
                _wallet(conn, "desc", "Descriptor", "descriptor", {"chain": "bitcoin", "network": "mainnet"})
                _wallet(conn, "old", "Old address list", "address", {"addresses": [ADDR_A], "chain": "bitcoin", "network": "mainnet", "deprecated": True})
                _utxo(conn, "desc", ADDR_A, txid="aa")
                conn.commit()
                blockers = build_report_blockers_snapshot(conn)
                self.assertNotIn("source_overlap", [item["id"] for item in blockers["blockers"]])

                _tx(conn, "tx-old", "old", "bb" * 32)
                conn.commit()
                blockers = build_report_blockers_snapshot(conn)
                source_blocker = next(item for item in blockers["blockers"] if item["id"] == "source_overlap")
                self.assertEqual(source_blocker["overlap"]["overlap_count"], 1)
                self.assertNotIn(_script(ADDR_A), json.dumps(source_blocker))
            finally:
                conn.close()

    def test_report_blocker_includes_preview_only_duplicate_transaction_repair(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-source-overlap-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                _seed_book(conn)
                _wallet(conn, "desc", "Descriptor", "descriptor", {"chain": "bitcoin", "network": "mainnet"})
                _wallet(conn, "addr", "Address list", "address", {"addresses": [ADDR_A], "chain": "bitcoin", "network": "mainnet"})
                _utxo(conn, "desc", ADDR_A, txid="aa")
                _tx(conn, "tx-desc", "desc", "cc" * 32)
                _tx(conn, "tx-addr", "addr", "cc" * 32)
                conn.commit()

                blockers = build_report_blockers_snapshot(conn)
                source_blocker = next(item for item in blockers["blockers"] if item["id"] == "source_overlap")
                preview = source_blocker["repair_preview"]
                self.assertEqual(preview["recommended_exclusions"], ["tx-addr"])
                self.assertIn("Preview only", preview["repair_policy"])
                overlap = source_blocker["overlap"]["overlaps"][0]
                self.assertEqual(overlap["recommended_canonical_wallet_id"], "desc")
                self.assertEqual(
                    overlap["address_list_repair_preview"],
                    [
                        {
                            "wallet_id": "addr",
                            "wallet": "Address list",
                            "overlapping_address_list_target_count": 1,
                            "action": "remove_overlapping_address_list_targets",
                            "clear_output_inventory": True,
                            "reset_onchain_refresh_checkpoint": True,
                            "deprecate_if_empty_after_trim": True,
                            "requires_confirmation": True,
                        }
                    ],
                )
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
