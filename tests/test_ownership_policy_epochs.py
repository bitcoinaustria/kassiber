from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from kassiber.core.chain_observer import CoveragePoint, ObserverIdentity
from kassiber.core.ownership_policy_epochs import (
    record_observer_policy_coverage,
    retired_policy_materials,
    roll_wallet_policy_epoch,
    technical_coverage_snapshot,
)
from kassiber.core.sync_replication.schema_allowlist import SYNC_TABLES
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
        self.assertEqual(snapshot["scope"], "imported_policy_technical_coverage")
        self.assertEqual({row["epoch_id"] for row in snapshot["epochs"]}, {epoch_id})
        self.assertEqual(
            {
                (row["branch_key"], row["scanned_to_exclusive"])
                for row in snapshot["epochs"]
            },
            {("receive", 20), ("change", 20)},
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

    def test_private_epoch_tables_are_not_replicated(self):
        self.assertTrue(
            {
                "wallet_policy_epochs",
                "wallet_policy_sources",
                "wallet_policy_coverage_witnesses",
            }.isdisjoint(SYNC_TABLES)
        )


if __name__ == "__main__":
    unittest.main()
