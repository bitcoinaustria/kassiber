import json
import sqlite3
import unittest

from kassiber.core.ownership_coverage import assess_profile_ownership_coverage
from kassiber.core.wallets import _validated_wallet_config
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

        coverage = assess_profile_ownership_coverage(conn, "profile")

        self.assertEqual(coverage.tier_for("bitcoin", "main"), "assumed")

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


if __name__ == "__main__":
    unittest.main()
