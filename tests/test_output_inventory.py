import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core.output_inventory import (
    clear_wallet_output_inventory,
    list_wallet_output_inventory,
    update_wallet_output_inventory,
    wallet_output_inventory_totals,
    wallet_output_inventory_summary,
)
from kassiber.core.sync import WalletSyncState
from kassiber.db import open_db
from kassiber.time_utils import timestamp_to_iso


def _seed_wallet(conn):
    now = "2026-01-01T00:00:00Z"
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        ("workspace-1", "Demo", now),
    )
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        ("profile-1", "workspace-1", "Main", "EUR", "generic", 365, "FIFO", now),
    )
    conn.execute(
        """
        INSERT INTO accounts(
            id, workspace_id, profile_id, code, label, account_type, asset, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        ("account-1", "workspace-1", "profile-1", "treasury", "Treasury", "asset", "BTC", now),
    )
    conn.execute(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "wallet-1",
            "workspace-1",
            "profile-1",
            "account-1",
            "Vault",
            "descriptor",
            json.dumps({"backend": "esplora", "chain": "bitcoin", "network": "mainnet"}),
            now,
        ),
    )
    conn.commit()
    profile = conn.execute("SELECT * FROM profiles WHERE id = 'profile-1'").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id = 'wallet-1'").fetchone()
    return profile, wallet


def _insert_wallet_utxo(
    conn,
    profile,
    wallet,
    *,
    row_id,
    txid,
    backend_name,
    backend_kind,
    chain="bitcoin",
    network="main",
    amount=25_000_000,
    spent_at=None,
):
    conn.execute(
        """
        INSERT INTO wallet_utxos(
            id, workspace_id, profile_id, wallet_id, backend_name,
            backend_kind, chain, network, asset, amount, txid, vout,
            outpoint, confirmation_status, confirmations, block_height,
            block_time, address, address_label, branch_label,
            branch_index, address_index, first_seen_at, last_seen_at,
            spent_at, raw_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            row_id,
            profile["workspace_id"],
            profile["id"],
            wallet["id"],
            backend_name,
            backend_kind,
            chain,
            network,
            "BTC",
            amount,
            txid,
            1,
            f"{txid}:1",
            "confirmed",
            10,
            809_000,
            "2026-01-01T11:00:00Z",
            "bc1qoldsource",
            "receive #9",
            "receive",
            0,
            9,
            "2026-01-01T11:00:00Z",
            "2026-01-01T11:00:00Z",
            spent_at,
            "{}",
        ),
    )


class OutputInventoryTest(unittest.TestCase):
    def test_refresh_marks_missing_outpoints_spent_only_for_same_source(self):
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="main",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[],
            tracked_scripts={},
            history_cache={},
        )
        old_txid = "aa" * 32
        current_txid = "bb" * 32
        with tempfile.TemporaryDirectory(prefix="kassiber-utxo-refresh-scope-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile, wallet = _seed_wallet(conn)
                _insert_wallet_utxo(
                    conn,
                    profile,
                    wallet,
                    row_id="old-source-active",
                    txid=old_txid,
                    backend_name="old-default",
                    backend_kind="electrum",
                )

                result = update_wallet_output_inventory(
                    conn,
                    profile,
                    wallet,
                    {"name": "current-default", "kind": "esplora"},
                    sync_state,
                    [
                        {
                            "txid": current_txid,
                            "vout": 0,
                            "amount_sats": 50_000,
                            "asset": "BTC",
                            "confirmation_status": "confirmed",
                            "block_height": 810_000,
                        }
                    ],
                    seen_at="2026-01-01T12:00:00Z",
                )

                self.assertEqual(result["spent"], 0)
                old_row = conn.execute(
                    "SELECT spent_at FROM wallet_utxos WHERE id = ?",
                    ("old-source-active",),
                ).fetchone()
                self.assertIsNone(old_row["spent_at"])
                old_summary = wallet_output_inventory_summary(
                    conn,
                    wallet["id"],
                    backend_name="old-default",
                    backend_kind="electrum",
                    chain="bitcoin",
                    network="main",
                )
                self.assertEqual(old_summary["active_count"], 1)
                current_summary = wallet_output_inventory_summary(
                    conn,
                    wallet["id"],
                    backend_name="current-default",
                    backend_kind="esplora",
                    chain="bitcoin",
                    network="main",
                )
                self.assertEqual(current_summary["active_count"], 1)
            finally:
                conn.close()

    def test_inventory_summary_and_rows_can_be_scoped_to_source(self):
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="main",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[],
            tracked_scripts={},
            history_cache={},
        )
        current_txid = "cc" * 32
        old_txid = "dd" * 32
        with tempfile.TemporaryDirectory(prefix="kassiber-utxo-source-filter-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile, wallet = _seed_wallet(conn)
                update_wallet_output_inventory(
                    conn,
                    profile,
                    wallet,
                    {"name": "current-default", "kind": "esplora"},
                    sync_state,
                    [
                        {
                            "txid": current_txid,
                            "vout": 0,
                            "amount_sats": 50_000,
                            "asset": "BTC",
                            "chain": "bitcoin",
                            "network": "main",
                            "confirmation_status": "confirmed",
                            "block_height": 810_000,
                        }
                    ],
                    seen_at="2026-01-01T12:00:00Z",
                )
                conn.execute(
                    """
                    INSERT INTO transactions(
                        id, workspace_id, profile_id, wallet_id, external_id,
                        fingerprint, occurred_at, confirmed_at, direction,
                        asset, amount, fee, fiat_currency, fiat_rate,
                        fiat_value, fiat_price_source, kind, description,
                        counterparty, note, excluded, raw_json, created_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        "tx-current-utxo",
                        profile["workspace_id"],
                        profile["id"],
                        wallet["id"],
                        current_txid,
                        "fp-current-utxo",
                        "2026-01-01T12:00:00Z",
                        "2026-01-01T12:00:00Z",
                        "inbound",
                        "BTC",
                        50_000_000,
                        0,
                        "EUR",
                        50_000,
                        25,
                        "manual",
                        "transfer",
                        "Funding",
                        "Exchange",
                        None,
                        0,
                        "{}",
                        "2026-01-01T12:00:00Z",
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO wallet_utxos(
                        id, workspace_id, profile_id, wallet_id, backend_name,
                        backend_kind, chain, network, asset, amount, txid, vout,
                        outpoint, confirmation_status, confirmations, block_height,
                        block_time, address, address_label, branch_label,
                        branch_index, address_index, first_seen_at, last_seen_at,
                        spent_at, raw_json
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        "stale-source-row",
                        profile["workspace_id"],
                        profile["id"],
                        wallet["id"],
                        "old-default",
                        "electrum",
                        "bitcoin",
                        "main",
                        "BTC",
                        25_000_000,
                        old_txid,
                        1,
                        f"{old_txid}:1",
                        "confirmed",
                        10,
                        809_000,
                        "2026-01-01T11:00:00Z",
                        "bc1qoldsource",
                        "receive #9",
                        "receive",
                        0,
                        9,
                        "2026-01-01T11:00:00Z",
                        "2026-01-01T11:00:00Z",
                        None,
                        "{}",
                    ),
                )
                conn.execute(
                    """
                    INSERT INTO wallet_utxo_refreshes(
                        wallet_id, workspace_id, profile_id, backend_name,
                        backend_kind, chain, network, observed_count,
                        active_count, last_seen_at
                    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(wallet_id) DO UPDATE SET
                        backend_name = excluded.backend_name,
                        backend_kind = excluded.backend_kind,
                        chain = excluded.chain,
                        network = excluded.network,
                        observed_count = excluded.observed_count,
                        active_count = excluded.active_count,
                        last_seen_at = excluded.last_seen_at
                    """,
                    (
                        wallet["id"],
                        profile["workspace_id"],
                        profile["id"],
                        "old-default",
                        "electrum",
                        "bitcoin",
                        "main",
                        1,
                        1,
                        "2026-01-01T13:00:00Z",
                    ),
                )
                conn.commit()

                self.assertEqual(
                    len(list_wallet_output_inventory(conn, wallet["id"])),
                    2,
                )
                rows = list_wallet_output_inventory(
                    conn,
                    wallet["id"],
                    backend_name="current-default",
                    backend_kind="esplora",
                    chain="bitcoin",
                    network=["main", "mainnet"],
                )
                self.assertEqual([row["outpoint"] for row in rows], [f"{current_txid}:0"])
                self.assertEqual(rows[0]["transaction_id"], "tx-current-utxo")
                summary = wallet_output_inventory_summary(
                    conn,
                    wallet["id"],
                    backend_name="current-default",
                    backend_kind="esplora",
                    chain="bitcoin",
                    network=["main", "mainnet"],
                )
                self.assertEqual(summary["active_count"], 1)
                self.assertEqual(summary["observed_count"], 0)
                self.assertEqual(summary["last_seen_at"], "2026-01-01T12:00:00Z")
            finally:
                conn.close()

    def test_inventory_rows_can_be_limited_without_truncating_totals(self):
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="mainnet",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[],
            tracked_scripts={},
            history_cache={},
        )
        with tempfile.TemporaryDirectory(prefix="kassiber-utxo-limit-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile, wallet = _seed_wallet(conn)
                outputs = [
                    {
                        "txid": f"{index:064x}",
                        "vout": 0,
                        "amount_sats": 1_000 + index,
                        "asset": "BTC",
                        "chain": "bitcoin",
                        "network": "mainnet",
                        "confirmation_status": "confirmed",
                        "block_height": 800_000 + index,
                    }
                    for index in range(1, 6)
                ]
                update_wallet_output_inventory(
                    conn,
                    profile,
                    wallet,
                    {"name": "renamed-mempool", "kind": "esplora"},
                    sync_state,
                    outputs,
                    seen_at="2026-01-01T12:00:00Z",
                )

                rows = list_wallet_output_inventory(
                    conn,
                    wallet["id"],
                    backend_kind="esplora",
                    chain="bitcoin",
                    network="mainnet",
                    limit=2,
                )
                self.assertEqual(len(rows), 2)
                totals = wallet_output_inventory_totals(
                    conn,
                    wallet["id"],
                    backend_kind="esplora",
                    chain="bitcoin",
                    network="mainnet",
                )
                self.assertEqual(
                    totals[0]["amount_sat"],
                    sum(1_000 + index for index in range(1, 6)),
                )
                unrenamed_summary = wallet_output_inventory_summary(
                    conn,
                    wallet["id"],
                    backend_kind="esplora",
                    chain="bitcoin",
                    network="mainnet",
                )
                self.assertEqual(unrenamed_summary["active_count"], 5)
                self.assertEqual(unrenamed_summary["observed_count"], 5)
                summary = wallet_output_inventory_summary(
                    conn,
                    wallet["id"],
                    backend_name="old-mempool-name",
                    backend_kind="esplora",
                    chain="bitcoin",
                    network="mainnet",
                )
                self.assertEqual(summary["active_count"], 0)
                self.assertEqual(summary["observed_count"], 0)
                self.assertIsNone(summary["last_seen_at"])
            finally:
                conn.close()

    def test_esplora_utxos_keep_derivation_metadata_and_spent_detection(self):
        from kassiber.core.sync_backends import esplora_utxos_for_wallet

        target_receive = {
            "chain": "bitcoin",
            "network": "mainnet",
            "branch_index": 0,
            "branch_label": "receive",
            "address_index": 0,
            "address": "bc1qreceive",
            "script_pubkey": "0014" + ("11" * 20),
        }
        target_change = {
            "chain": "bitcoin",
            "network": "mainnet",
            "branch_index": 1,
            "branch_label": "change",
            "address_index": 7,
            "address": "bc1qchange",
            "script_pubkey": "0014" + ("22" * 20),
        }
        sync_state = WalletSyncState(
            chain="bitcoin",
            network="mainnet",
            descriptor_plan=None,
            policy_asset_id="",
            targets=[target_receive, target_change],
            tracked_scripts={
                target_receive["script_pubkey"]: target_receive,
                target_change["script_pubkey"]: target_change,
            },
            history_cache={},
        )
        receive_txid = "aa" * 32
        change_txid = "bb" * 32

        def fake_fetch(_base_url, script_pubkey, timeout=30, proxy_url=None):
            del timeout, proxy_url
            if script_pubkey == target_receive["script_pubkey"]:
                return [
                    {
                        "txid": receive_txid,
                        "vout": 0,
                        "value": 12_345,
                        "status": {
                            "confirmed": True,
                            "block_height": 800_000,
                            "block_time": 1_700_000_000,
                        },
                    }
                ]
            if script_pubkey == target_change["script_pubkey"]:
                return [
                    {
                        "txid": change_txid,
                        "vout": 2,
                        "value": 98_765,
                        "status": {"confirmed": False},
                    }
                ]
            return []

        with patch(
            "kassiber.core.sync_backends.fetch_esplora_scripthash_utxos",
            side_effect=fake_fetch,
        ), patch(
            "kassiber.core.sync_backends.http_get_text",
            return_value="800002\n",
        ):
            outputs = esplora_utxos_for_wallet(
                {"name": "esplora", "kind": "esplora", "url": "https://example.invalid"},
                sync_state,
            )

        self.assertEqual(len(outputs), 2)
        self.assertEqual(outputs[0]["address_label"], "receive #0")
        self.assertEqual(outputs[0]["confirmations"], 3)
        self.assertEqual(outputs[0]["block_time"], timestamp_to_iso(1_700_000_000))
        self.assertEqual(outputs[1]["branch_label"], "change")
        self.assertEqual(outputs[1]["address_index"], 7)
        self.assertIsNone(outputs[1]["confirmations"])

        with tempfile.TemporaryDirectory(prefix="kassiber-utxos-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                profile, wallet = _seed_wallet(conn)
                first = update_wallet_output_inventory(
                    conn,
                    profile,
                    wallet,
                    {"name": "esplora", "kind": "esplora"},
                    sync_state,
                    outputs,
                    seen_at="2026-01-01T12:00:00Z",
                )
                self.assertEqual(first["observed"], 2)
                active = list_wallet_output_inventory(conn, wallet["id"])
                self.assertEqual([row["outpoint"] for row in active], [f"{receive_txid}:0", f"{change_txid}:2"])
                self.assertEqual(active[0]["amount_sat"], 12_345)
                self.assertEqual(active[0]["branch_label"], "receive")

                second = update_wallet_output_inventory(
                    conn,
                    profile,
                    wallet,
                    {"name": "esplora", "kind": "esplora"},
                    sync_state,
                    [outputs[1]],
                    seen_at="2026-01-01T13:00:00Z",
                )
                self.assertEqual(second["spent"], 1)
                active = list_wallet_output_inventory(conn, wallet["id"])
                self.assertEqual([row["outpoint"] for row in active], [f"{change_txid}:2"])
                all_rows = list_wallet_output_inventory(
                    conn,
                    wallet["id"],
                    include_spent=True,
                )
                spent = [row for row in all_rows if row["source"]["spent_at"]]
                self.assertEqual(len(spent), 1)
                self.assertEqual(spent[0]["outpoint"], f"{receive_txid}:0")
                self.assertEqual(
                    spent[0]["source"]["last_seen_at"],
                    "2026-01-01T12:00:00Z",
                )

                third = update_wallet_output_inventory(
                    conn,
                    profile,
                    wallet,
                    {"name": "esplora", "kind": "esplora"},
                    sync_state,
                    [],
                    seen_at="2026-01-01T14:00:00Z",
                )
                self.assertEqual(third["observed"], 0)
                self.assertEqual(third["spent"], 1)
                self.assertEqual(list_wallet_output_inventory(conn, wallet["id"]), [])
                summary = wallet_output_inventory_summary(conn, wallet["id"])
                self.assertEqual(summary["active_count"], 0)
                self.assertEqual(summary["observed_count"], 0)
                self.assertEqual(summary["last_seen_at"], "2026-01-01T14:00:00Z")
                all_rows = list_wallet_output_inventory(
                    conn,
                    wallet["id"],
                    include_spent=True,
                )
                spent_by_outpoint = {
                    row["outpoint"]: row
                    for row in all_rows
                    if row["source"]["spent_at"]
                }
                self.assertEqual(
                    spent_by_outpoint[f"{change_txid}:2"]["source"]["last_seen_at"],
                    "2026-01-01T13:00:00Z",
                )

                cleared = clear_wallet_output_inventory(conn, wallet["id"])
                self.assertGreaterEqual(cleared["utxos_deleted"], 1)
                self.assertEqual(cleared["refreshes_deleted"], 1)
                self.assertEqual(list_wallet_output_inventory(conn, wallet["id"]), [])
                summary = wallet_output_inventory_summary(conn, wallet["id"])
                self.assertEqual(summary["active_count"], 0)
                self.assertEqual(summary["last_seen_at"], None)
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
