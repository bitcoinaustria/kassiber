import json
import sqlite3
import unittest

from kassiber.core.ownership_coverage import (
    assess_profile_ownership_coverage,
    attest_profile_wallet_universe,
    build_ownership_coverage_snapshot,
    clear_profile_wallet_universe_attestation,
)
from kassiber.core.wallets import (
    _ownership_material_identity_snapshot,
    _sync_material_config_json,
    _validated_wallet_config,
)
from kassiber.errors import AppError


def _conn():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE wallets(
            id TEXT, profile_id TEXT, label TEXT, kind TEXT, config_json TEXT
        );
        CREATE TABLE freshness_source_states(
            profile_id TEXT, source_key TEXT, checkpoint_json TEXT
        );
        CREATE TABLE profiles(
            id TEXT PRIMARY KEY,
            last_processed_at TEXT,
            last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
            journal_input_version INTEGER NOT NULL DEFAULT 0,
            ownership_review_counts_json TEXT
        );
        CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT);
        INSERT INTO profiles(id, last_processed_at, last_processed_tx_count,
                             journal_input_version, ownership_review_counts_json)
        VALUES('profile', '2026-01-01T00:00:00Z', 3, 7, '{}');
        """
    )
    return conn


def _wallet(conn, wallet_id, label, config):
    conn.execute(
        "INSERT INTO wallets VALUES(?, 'profile', ?, 'address', ?)",
        (wallet_id, label, json.dumps(config)),
    )


class OwnershipCoverageTests(unittest.TestCase):
    def test_authoritative_finite_policy_is_proven(self):
        conn = _conn()
        _wallet(
            conn,
            "vault",
            "Vault",
            {
                "addresses": ["bc1qowned"],
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "branch_last_issued": {},
                },
            },
        )

        coverage = assess_profile_ownership_coverage(conn, "profile")
        self.assertFalse(coverage.is_policy_proven("bitcoin", "main"))
        attest_profile_wallet_universe(conn, "profile", complete=True)
        coverage = assess_profile_ownership_coverage(conn, "profile")

        self.assertTrue(coverage.is_policy_proven("bitcoin", "main"))
        self.assertEqual(coverage.wallets[0].history_tier, "unknown")

    def test_user_attestation_stays_assumed(self):
        conn = _conn()
        _wallet(
            conn,
            "vault",
            "Vault",
            {
                "addresses": ["bc1qowned"],
                "ownership_policy": {
                    "complete": True,
                    "evidence": "user_attested",
                    "branch_last_issued": {},
                },
            },
        )

        attest_profile_wallet_universe(conn, "profile", complete=True)
        coverage = assess_profile_ownership_coverage(conn, "profile")

        self.assertEqual(coverage.tier_for("bitcoin", "main"), "assumed")

    def test_wallet_universe_attestation_invalidates_journals(self):
        conn = _conn()

        attest_profile_wallet_universe(conn, "profile", complete=True)

        profile = conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        self.assertIsNone(profile["last_processed_at"])
        self.assertEqual(profile["last_processed_tx_count"], 0)
        self.assertEqual(profile["journal_input_version"], 8)
        self.assertIsNone(profile["ownership_review_counts_json"])

    def test_wallet_universe_attestation_can_be_revoked(self):
        conn = _conn()
        attest_profile_wallet_universe(conn, "profile", complete=True)

        changed = clear_profile_wallet_universe_attestation(conn, "profile")

        self.assertTrue(changed)
        coverage = assess_profile_ownership_coverage(conn, "profile")
        self.assertFalse(coverage.universe_complete)

    def test_filtered_wallet_summary_respects_other_policies_in_scope(self):
        conn = _conn()
        for wallet_id, evidence in (("proven", "wallet_export"), ("assumed", "user_attested")):
            _wallet(
                conn,
                wallet_id,
                wallet_id.title(),
                {
                    "addresses": [f"bc1q{wallet_id}"],
                    "ownership_policy": {
                        "complete": True,
                        "evidence": evidence,
                        "branch_last_issued": {},
                    },
                },
            )
        attest_profile_wallet_universe(conn, "profile", complete=True)

        snapshot = build_ownership_coverage_snapshot(
            conn, "profile", wallet_id="proven"
        )

        self.assertTrue(snapshot["summary"]["all_policy_proven"])
        self.assertFalse(snapshot["summary"]["effective_policy_proven"])

    def test_wildcard_policy_needs_branch_bounds(self):
        conn = _conn()
        _wallet(
            conn,
            "vault",
            "Vault",
            {
                "descriptor": "wpkh(xpub/example/*)",
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "branch_last_issued": {},
                },
            },
        )

        wallet = assess_profile_ownership_coverage(conn, "profile").wallets[0]

        self.assertEqual(wallet.policy_tier, "unknown")
        self.assertIn("wildcard_branch_bounds_missing", wallet.limitations)

    def test_config_validation_raises_derivation_floor_to_declared_bound(self):
        config = _validated_wallet_config(
            "custom",
            {
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "branch_last_issued": {"0": 42, "1": 81},
                }
            },
        )

        self.assertEqual(config["ownership_scan_to_index"], 81)

    def test_invalid_policy_evidence_is_rejected(self):
        with self.assertRaises(AppError) as raised:
            _validated_wallet_config(
                "custom",
                {
                    "ownership_policy": {
                        "complete": True,
                        "evidence": "wishful",
                    }
                },
            )
        self.assertEqual(raised.exception.code, "validation")

    def test_coverage_metadata_does_not_change_wallet_material_identity(self):
        original = {"descriptor": "wpkh(xpub/example/*)", "gap_limit": 20}
        covered = {
            **original,
            "ownership_scan_to_index": 81,
            "ownership_policy": {
                "complete": True,
                "evidence": "wallet_export",
                "branch_last_issued": {"0": 81},
            },
        }

        self.assertEqual(
            _ownership_material_identity_snapshot(original),
            _ownership_material_identity_snapshot(covered),
        )
        self.assertEqual(
            _sync_material_config_json(original),
            _sync_material_config_json(covered),
        )

    def test_multisig_policy_is_supported_and_grouped(self):
        conn = _conn()
        _wallet(
            conn,
            "vault",
            "Vault",
            {
                "descriptor": "wsh(sortedmulti(2,xpubA/0/*,xpubB/0/*))",
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "policy_set_id": "family-vault",
                    "branch_last_issued": {"0": 12},
                },
            },
        )

        coverage = assess_profile_ownership_coverage(
            conn,
            "profile",
            derived_through_by_wallet={"vault": {"0": 12}},
        ).wallets[0]

        self.assertEqual(coverage.policy_tier, "proven")
        self.assertEqual(coverage.policy_shape, "multisig_descriptor")
        self.assertEqual(coverage.policy_set_id, "family-vault")

    def test_incomplete_historic_wildcard_policy_blocks_proof(self):
        conn = _conn()
        _wallet(
            conn,
            "vault",
            "Vault",
            {
                "addresses": ["bc1qcurrent"],
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "branch_last_issued": {},
                },
                "ownership_history": [
                    {"descriptor": "wpkh(xpub/old/*)", "scan_to_index": 50}
                ],
            },
        )
        attest_profile_wallet_universe(conn, "profile", complete=True)

        wallet = assess_profile_ownership_coverage(conn, "profile").wallets[0]

        self.assertEqual(wallet.policy_tier, "unknown")
        self.assertIn("historic_policy_coverage_missing", wallet.limitations)

    def test_failed_historic_derivation_blocks_aggregated_depth_proof(self):
        conn = _conn()
        _wallet(
            conn,
            "vault",
            "Vault",
            {
                "descriptor": "wpkh(xpub/current/*)",
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "branch_last_issued": {"0": 20},
                },
                "ownership_history": [
                    {
                        "descriptor": "wpkh(xpub/old/*)",
                        "ownership_policy": {
                            "complete": True,
                            "evidence": "wallet_export",
                            "branch_last_issued": {"0": 10},
                        },
                    }
                ],
            },
        )
        attest_profile_wallet_universe(conn, "profile", complete=True)

        wallet = assess_profile_ownership_coverage(
            conn,
            "profile",
            derived_through_by_wallet={"vault": {"0": 20}},
            derivation_complete_by_wallet={"vault": False},
        ).wallets[0]

        self.assertEqual(wallet.policy_tier, "unknown")
        self.assertIn("wallet_policy_derivation_incomplete", wallet.limitations)

    def test_silent_payment_requires_completed_full_history_scan(self):
        conn = _conn()
        _wallet(
            conn,
            "silent",
            "Silent",
            {
                "sp_descriptor": "sp(spscan1qexample)",
                "sp_full_history": True,
                "ownership_policy": {
                    "complete": True,
                    "evidence": "wallet_export",
                    "branch_last_issued": {},
                },
            },
        )
        attest_profile_wallet_universe(conn, "profile", complete=True)

        before = assess_profile_ownership_coverage(conn, "profile").wallets[0]
        self.assertEqual(before.policy_tier, "unknown")
        conn.execute(
            "INSERT INTO freshness_source_states VALUES(?, ?, ?)",
            (
                "profile",
                "onchain_wallet:silent",
                json.dumps(
                    {
                        "backend": {"kind": "custom"},
                        "silent_payment": {
                            "scan_complete": True,
                            "degraded": False,
                            "full_history": True,
                        },
                    }
                ),
            ),
        )

        after = assess_profile_ownership_coverage(conn, "profile").wallets[0]
        self.assertEqual(after.policy_tier, "proven")
        self.assertEqual(after.history_tier, "proven")


if __name__ == "__main__":
    unittest.main()
