from dataclasses import replace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from kassiber.core.accounts import (
    create_profile,
    create_workspace,
    get_profile_details,
    update_profile,
)
from kassiber.core import wallets as core_wallets
from kassiber.cli.handlers import process_journals
from kassiber.cli.main import build_parser
from kassiber.core.sync_replication.schema_allowlist import SYNC_TABLE_MAP
from kassiber.core.sync_replication.crypto import encode_secret
from kassiber.core.sync_replication.merge import _prepare_actual_row
from kassiber.core.ui_snapshot import (
    build_profiles_snapshot,
    build_report_blockers_snapshot,
)
from kassiber.db import open_db
from kassiber.daemon import _create_profile_payload, _update_profile_payload
from kassiber.errors import AppError
from kassiber.tax_policy import (
    POLICY_BUILDERS,
    build_generic_policy,
    resolve_cost_basis_pool_id,
)


class CostBasisPoolPolicyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-pool-policy-")
        self.data_root = Path(self.temp.name) / "data"
        self.conn = open_db(self.data_root)
        create_workspace(self.conn, "Books")
        self.profile = create_profile(
            self.conn, "Books", "Main", "EUR", "FIFO", "generic", 365
        )

    def tearDown(self):
        self.conn.close()
        self.temp.cleanup()

    def test_default_round_trips_and_wallet_scope_fails_closed(self):
        details = get_profile_details(self.conn, "Books", "Main")
        self.assertEqual(details["cost_basis_pool_scope"], "global")
        self.assertEqual(details["allowed_cost_basis_pool_scopes"], ["global"])
        snapshot_profile = build_profiles_snapshot(self.conn)["workspaces"][0]["profiles"][0]
        self.assertEqual(snapshot_profile["costBasisPoolScope"], "global")
        self.assertEqual(snapshot_profile["allowedCostBasisPoolScopes"], ["global"])
        self.assertEqual(resolve_cost_basis_pool_id(self.profile, "wallet-a"), "global")

        with self.assertRaises(AppError) as raised:
            update_profile(
                self.conn,
                "Books",
                "Main",
                {"cost_basis_pool_scope": "wallet"},
            )
        self.assertEqual(raised.exception.code, "validation")

    def test_cli_defers_scope_validation_to_country_policy(self):
        args = build_parser().parse_args(
            ["profiles", "set", "--cost-basis-pool-scope", "wallet"]
        )
        self.assertEqual(args.cost_basis_pool_scope, "wallet")

    def test_daemon_create_copy_and_update_round_trip_global(self):
        created = _create_profile_payload(
            self.conn,
            {
                "workspace_id": self.profile["workspace_id"],
                "label": "Copy",
                "source_profile_id": self.profile["id"],
            },
        )
        copied_id = created["activeProfileId"]
        copied = self.conn.execute(
            "SELECT cost_basis_pool_scope FROM profiles WHERE id = ?", (copied_id,)
        ).fetchone()
        self.assertEqual(copied["cost_basis_pool_scope"], "global")

        updated = _update_profile_payload(
            self.conn,
            {"profile_id": copied_id, "cost_basis_pool_scope": "global"},
        )
        self.assertEqual(updated["cost_basis_pool_scope"], "global")

    def test_supported_scope_change_blocks_reports_until_reprocessed(self):
        def wallet_test_policy(profile):
            return replace(
                build_generic_policy(profile),
                allowed_cost_basis_pool_scopes=("global", "wallet"),
            )

        wallet = core_wallets.create_wallet(
            self.conn,
            self.profile["workspace_id"],
            self.profile["id"],
            "Policy fixture",
            "custom",
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_rate_exact, raw_json, created_at
            ) VALUES(
                'pool-policy-in', ?, ?, ?, 'pool-policy-in',
                'fp-pool-policy-in', '2026-01-01T00:00:00Z', 'inbound',
                'BTC', 100000000000, 0, 'EUR', 10000, '10000', '{}',
                '2026-01-01T00:00:00Z'
            )
            """,
            (self.profile["workspace_id"], self.profile["id"], wallet["id"]),
        )
        self.conn.commit()

        with patch.dict(POLICY_BUILDERS, {"generic": wallet_test_policy}):
            processed = process_journals(self.conn, "Books", "Main")
            self.assertEqual(processed["processed_transactions"], 1)
            self.assertNotIn(
                "journals_stale",
                {
                    blocker["id"]
                    for blocker in build_report_blockers_snapshot(self.conn)["blockers"]
                },
            )
            version_before_scope_change = self.conn.execute(
                "SELECT journal_input_version FROM profiles WHERE id = ?",
                (self.profile["id"],),
            ).fetchone()[0]

            updated = update_profile(
                self.conn,
                "Books",
                "Main",
                {"cost_basis_pool_scope": "wallet"},
            )
            self.assertEqual(updated["cost_basis_pool_scope"], "wallet")
            self.assertEqual(resolve_cost_basis_pool_id(updated, "wallet-a"), "wallet-a")

            stale = build_report_blockers_snapshot(self.conn)
            self.assertIn(
                "journals_stale", {blocker["id"] for blocker in stale["blockers"]}
            )

            reprocessed = process_journals(self.conn, "Books", "Main")
            self.assertEqual(reprocessed["processed_transactions"], 1)
            refreshed = build_report_blockers_snapshot(self.conn)
            self.assertNotIn(
                "journals_stale",
                {blocker["id"] for blocker in refreshed["blockers"]},
            )

        row = self.conn.execute(
            "SELECT journal_input_version, last_processed_at, last_processed_tx_count, "
            "last_processed_input_version "
            "FROM profiles WHERE id = ?",
            (self.profile["id"],),
        ).fetchone()
        self.assertEqual(
            row["journal_input_version"], version_before_scope_change + 1
        )
        self.assertIsNotNone(row["last_processed_at"])
        self.assertEqual(row["last_processed_tx_count"], 1)
        self.assertEqual(
            row["last_processed_input_version"], row["journal_input_version"]
        )

    def test_replication_contract_is_additive_and_high_stakes(self):
        spec = SYNC_TABLE_MAP["profiles"]
        self.assertIn("cost_basis_pool_scope", spec.columns)
        self.assertIn("cost_basis_pool_scope", spec.optional_columns)
        self.assertIn("cost_basis_pool_scope", spec.high_stakes_fields)

        legacy_wire_row = {
            "id": "legacy-profile",
            "workspace_id": self.profile["workspace_id"],
            "label": "Legacy peer",
            "fiat_currency": "EUR",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
            "require_coarse_review": 0,
            "bitcoin_rail_carrying_value": 1,
            "created_at": "2026-01-01T00:00:00Z",
        }
        actual, _wire_pk = _prepare_actual_row(
            self.conn,
            book={
                "profile_id": self.profile["id"],
                "hmac_key_b64": encode_secret(b"pool-policy-test-key"),
            },
            spec=spec,
            wire_row=legacy_wire_row,
            blobs={},
            attachments_root=None,
            created_files=[],
        )
        self.assertEqual(actual["cost_basis_pool_scope"], "global")

    def test_legacy_profile_migrates_to_global(self):
        profile_id = self.profile["id"]
        self.conn.execute("ALTER TABLE profiles DROP COLUMN cost_basis_pool_scope")
        self.conn.commit()
        self.conn.close()

        self.conn = open_db(self.data_root)
        row = self.conn.execute(
            "SELECT cost_basis_pool_scope FROM profiles WHERE id = ?", (profile_id,)
        ).fetchone()
        self.assertEqual(row["cost_basis_pool_scope"], "global")


if __name__ == "__main__":
    unittest.main()
