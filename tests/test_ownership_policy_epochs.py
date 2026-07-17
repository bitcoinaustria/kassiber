from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from kassiber.core.chain_observer import CoveragePoint, ObserverIdentity
from kassiber.core.chain_observer.store import PRIVATE_OBSERVER_TABLES
from kassiber.core.ownership_policy_epochs import (
    canonical_wallet_config_identity,
    policy_identity_material,
    private_policy_material,
    record_observer_policy_coverage,
    retired_policy_materials,
    roll_wallet_policy_epoch,
    technical_coverage_snapshot,
)
from kassiber.core.sync_replication.schema_allowlist import (
    NEVER_SYNC_TABLES,
    SYNC_TABLE_MAP,
)
from kassiber.db import open_db
from kassiber.time_utils import now_iso


class OwnershipPolicyEpochTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-policy-epoch-")
        self.addCleanup(self.temp.cleanup)
        self.conn = open_db(Path(self.temp.name) / "data")
        self.addCleanup(self.conn.close)
        timestamp = now_iso()
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)",
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile', 'ws', 'Profile', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES('wallet', 'ws', 'profile', 'Cold', 'descriptor', ?, ?)
            """,
            (
                json.dumps(
                    {
                        "chain": "bitcoin",
                        "network": "main",
                        "descriptor": "old-public-descriptor",
                    }
                ),
                timestamp,
            ),
        )
        self.wallet = self.conn.execute(
            "SELECT * FROM wallets WHERE id = 'wallet'"
        ).fetchone()
        self.identity = ObserverIdentity(
            id="observer-structural-id",
            workspace_id="ws",
            profile_id="profile",
            logical_wallet_id="wallet",
            source_wallet_id="wallet",
            source_key="descriptor:default",
            observer_kind="bdk",
            chain="bitcoin",
            network="main",
            branch_keys=("receive", "change"),
        )

    def test_coverage_is_epoch_scoped_and_never_claims_wallet_universe(self):
        epoch_id = record_observer_policy_coverage(
            self.conn,
            self.identity,
            (
                CoveragePoint("receive", scanned_to=20, highest_used=2),
                CoveragePoint("change", scanned_to=20, highest_used=None),
            ),
        )

        snapshot = technical_coverage_snapshot(self.conn, "profile")
        self.assertFalse(snapshot["ownership_universe_known"])
        self.assertFalse(snapshot["coverage_can_clear_custody_gaps"])
        self.assertEqual(snapshot["scope"], "imported_policy_technical_coverage")
        self.assertEqual(snapshot["summary"]["wallet_count"], 1)
        self.assertEqual(snapshot["summary"]["covered_branch_count"], 2)
        wallet = snapshot["wallets"][0]
        self.assertEqual(wallet["wallet_label"], "Cold")
        self.assertEqual({row["epoch_id"] for row in wallet["epochs"]}, {epoch_id})
        source = wallet["epochs"][0]["sources"][0]
        self.assertEqual(source["source"], "descriptor-policy")
        self.assertEqual(source["observer_kind"], "bdk")
        self.assertEqual(
            {
                (row["branch"], row["scanned_to_exclusive"])
                for row in source["branches"]
            },
            {("receive", 20), ("change", 20)},
        )
        serialized = json.dumps(snapshot, sort_keys=True)
        for private_value in (
            "old-public-descriptor",
            "private_material_json",
            "source_key",
            "wallet_id",
            "descriptor:default",
            "observer-structural-id",
        ):
            self.assertNotIn(private_value, serialized)

    def test_comparison_identity_canonicalizes_aliases_without_rewriting_material(self):
        authored = {
            "chain": "btc",
            "network": "mainnet",
            "xpub": "xpub-public-material",
            "script_types": ["p2tr", "p2wpkh", "p2tr"],
        }
        represented = {
            "chain": "bitcoin",
            "network": "main",
            "xpub": "xpub-public-material",
            "script_types": ["p2wpkh", "p2tr"],
        }

        self.assertEqual(
            canonical_wallet_config_identity(authored),
            canonical_wallet_config_identity(represented),
        )
        self.assertEqual(
            policy_identity_material(authored),
            policy_identity_material(represented),
        )
        self.assertNotEqual(
            private_policy_material(authored),
            private_policy_material(represented),
        )

    def test_rollover_preserves_private_material_and_final_coverage(self):
        old_epoch_id = record_observer_policy_coverage(
            self.conn,
            self.identity,
            (CoveragePoint("receive", scanned_to=50, highest_used=7),),
        )
        retired_id, new_epoch_id = roll_wallet_policy_epoch(
            self.conn,
            self.wallet,
            {"chain": "bitcoin", "network": "main", "descriptor": "old-public-descriptor"},
            {"chain": "bitcoin", "network": "main", "descriptor": "new-public-descriptor"},
        )

        self.assertEqual(retired_id, old_epoch_id)
        self.assertNotEqual(new_epoch_id, old_epoch_id)
        rows = self.conn.execute(
            "SELECT id, status FROM wallet_policy_epochs ORDER BY created_at, id"
        ).fetchall()
        self.assertEqual(
            {row["id"]: row["status"] for row in rows},
            {old_epoch_id: "retired", new_epoch_id: "active"},
        )
        self.assertEqual(
            retired_policy_materials(self.conn, "wallet"),
            (
                {
                    "chain": "bitcoin",
                    "descriptor": "old-public-descriptor",
                    "network": "main",
                },
            ),
        )
        witness = self.conn.execute(
            """
            SELECT coverage.scanned_to_exclusive
            FROM wallet_policy_coverage_witnesses coverage
            JOIN wallet_policy_sources source ON source.id = coverage.source_id
            WHERE source.epoch_id = ? AND coverage.branch_key = 'receive'
            """,
            (old_epoch_id,),
        ).fetchone()
        self.assertEqual(witness["scanned_to_exclusive"], 50)

        snapshot = technical_coverage_snapshot(self.conn, "profile")
        timeline = snapshot["wallets"][0]["epochs"]
        self.assertEqual(snapshot["summary"]["active_epoch_count"], 1)
        self.assertEqual(snapshot["summary"]["retired_epoch_count"], 1)
        self.assertEqual(
            {epoch["epoch_id"]: epoch["status"] for epoch in timeline},
            {old_epoch_id: "retired", new_epoch_id: "active"},
        )
        retired = next(epoch for epoch in timeline if epoch["status"] == "retired")
        active = next(epoch for epoch in timeline if epoch["status"] == "active")
        self.assertIsNotNone(retired["retired_at"])
        self.assertEqual(
            retired["sources"][0]["branches"][0]["scanned_to_exclusive"],
            50,
        )
        self.assertEqual(active["sources"], [])
        serialized = json.dumps(snapshot, sort_keys=True)
        self.assertNotIn("old-public-descriptor", serialized)
        self.assertNotIn("new-public-descriptor", serialized)

    def test_open_migrates_inline_history_to_retired_epochs_once(self):
        config = json.loads(self.wallet["config_json"])
        config["ownership_history"] = [
            {
                "chain": "btc",
                "network": "mainnet",
                "descriptor": "retired-public-descriptor",
                "scan_to_index": 42,
            }
        ]
        self.conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = 'wallet'",
            (json.dumps(config, sort_keys=True),),
        )
        self.conn.commit()
        self.conn.close()

        data_root = Path(self.temp.name) / "data"
        self.conn = open_db(data_root)
        self.addCleanup(self.conn.close)
        stored = json.loads(
            self.conn.execute(
                "SELECT config_json FROM wallets WHERE id = 'wallet'"
            ).fetchone()["config_json"]
        )
        self.assertNotIn("ownership_history", stored)
        self.assertEqual(
            retired_policy_materials(self.conn, "wallet"),
            (
                {
                    "chain": "btc",
                    "descriptor": "retired-public-descriptor",
                    "network": "mainnet",
                    "ownership_scan_to_index": 42,
                },
            ),
        )
        self.conn.close()

        self.conn = open_db(data_root)
        self.addCleanup(self.conn.close)
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM wallet_policy_epochs "
                "WHERE wallet_id = 'wallet' AND status = 'retired'"
            ).fetchone()[0],
            1,
        )

    def test_snapshot_redacts_samourai_paths_and_unknown_observer_names(self):
        identity = ObserverIdentity(
            id="private-observer-id",
            workspace_id="ws",
            profile_id="profile",
            logical_wallet_id="wallet",
            source_wallet_id="wallet",
            source_key="samourai:postmix:p2wpkh:m/84'/0'/2147483646'",
            observer_kind="bdk:private-backend-name",
            chain="bitcoin",
            network="main",
            branch_keys=("receive",),
        )
        record_observer_policy_coverage(
            self.conn,
            identity,
            (CoveragePoint("receive", scanned_to=100, highest_used=11),),
        )

        snapshot = technical_coverage_snapshot(self.conn, "profile")
        source = snapshot["wallets"][0]["epochs"][0]["sources"][0]
        self.assertEqual(source["source"], "samourai:postmix")
        self.assertEqual(source["observer_kind"], "observer")
        serialized = json.dumps(snapshot, sort_keys=True)
        self.assertNotIn("2147483646", serialized)
        self.assertNotIn("private-backend-name", serialized)
        self.assertNotIn("p2wpkh", serialized)

    def test_private_epoch_tables_are_not_replicated(self):
        private_policy_tables = {
            "wallet_policy_epochs",
            "wallet_policy_sources",
            "wallet_policy_coverage_witnesses",
        }
        self.assertTrue(private_policy_tables <= NEVER_SYNC_TABLES)
        self.assertTrue(private_policy_tables.isdisjoint(SYNC_TABLE_MAP))
        self.assertTrue(PRIVATE_OBSERVER_TABLES <= NEVER_SYNC_TABLES)
        self.assertTrue(
            PRIVATE_OBSERVER_TABLES.isdisjoint(SYNC_TABLE_MAP)
        )


if __name__ == "__main__":
    unittest.main()
