import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core.output_inventory import (
    clear_wallet_output_inventory,
    list_wallet_output_inventory,
    update_wallet_output_inventory,
    wallet_output_inventory_summary,
)
from kassiber.core.sync import WalletSyncState
from kassiber.core.sync_backends import esplora_utxos_for_wallet
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


class OutputInventoryTest(unittest.TestCase):
    def test_esplora_utxos_keep_derivation_metadata_and_spent_detection(self):
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

        def fake_fetch(_base_url, script_pubkey, timeout=30):
            del timeout
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
        ):
            outputs = esplora_utxos_for_wallet(
                {"name": "esplora", "kind": "esplora", "url": "https://example.invalid"},
                sync_state,
            )

        self.assertEqual(len(outputs), 2)
        self.assertEqual(outputs[0]["address_label"], "receive #0")
        self.assertEqual(outputs[0]["block_time"], timestamp_to_iso(1_700_000_000))
        self.assertEqual(outputs[1]["branch_label"], "change")
        self.assertEqual(outputs[1]["address_index"], 7)

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
