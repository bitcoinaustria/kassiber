"""End-to-end CLI pin for swap-matching verbs.

Walks through ``transfers suggest`` / ``bulk-pair`` / ``dismiss``,
``transfers rules {list,create,apply,delete,enable,disable}``, and
``views {list,create,delete}`` against a temp data root with a
Phoenix LN row + a synthetic Liquid inbound that lines up by time
and amount. Pins the envelope ``kind`` + key fields so downstream
consumers (daemon, AI tools, UI) can trust the wire contract.
"""

import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


_PHOENIX_CSV = """date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description
2026-03-14T17:30:00Z,11111111-aaaa-bbbb-cccc-000000000001,lightning_sent,-100000000,-40.00 USD,0,0,0 USD,0,0 USD,abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789,,03somenode,LN to swap
"""

_LIQUID_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-14T17:32:00Z,liquid-claim-1,inbound,LBTC,0.000995,0,40000,Boltz claim
"""


def _run(data_root, *args):
    cmd = [
        sys.executable,
        "-m",
        "kassiber",
        "--data-root",
        str(data_root),
        "--machine",
        *args,
    ]
    result = subprocess.run(
        cmd, cwd=ROOT, capture_output=True, text=True, check=False
    )
    if not result.stdout.strip():
        raise AssertionError(
            f"CLI produced no stdout.\nargs: {args}\nstderr: {result.stderr}"
        )
    return json.loads(result.stdout), result.returncode


def _run_raw(data_root, *args):
    cmd = [
        sys.executable,
        "-m",
        "kassiber",
        "--data-root",
        str(data_root),
        *args,
    ]
    return subprocess.run(
        cmd, cwd=ROOT, capture_output=True, text=True, check=False
    )


def _bootstrap_profile(data_root, phoenix_csv, liquid_csv):
    """Initialise the temp data root with Phoenix + Liquid wallets pre-loaded."""
    _run(data_root, "init")
    _run(data_root, "workspaces", "create", "Main")
    _run(
        data_root, "profiles", "create",
        "--workspace", "Main",
        "--fiat-currency", "USD",
        "--tax-country", "at",
        "Swap",
    )
    _run(
        data_root, "wallets", "create",
        "--workspace", "Main",
        "--profile", "Swap",
        "--label", "phoenix-ln",
        "--kind", "phoenix",
    )
    _run(
        data_root, "wallets", "create",
        "--workspace", "Main",
        "--profile", "Swap",
        "--label", "liquid-onchain",
        "--kind", "custom",
    )
    _run(
        data_root, "wallets", "import-phoenix",
        "--workspace", "Main",
        "--profile", "Swap",
        "--wallet", "phoenix-ln",
        "--file", str(phoenix_csv),
    )
    _run(
        data_root, "wallets", "import-csv",
        "--workspace", "Main",
        "--profile", "Swap",
        "--wallet", "liquid-onchain",
        "--file", str(liquid_csv),
    )


def _mark_journals_processed(data_root):
    """Mark the fixture profile as current when a test targets report shaping."""
    conn = sqlite3.connect(data_root / "kassiber.sqlite3")
    try:
        profile = conn.execute(
            "SELECT id, journal_input_version FROM profiles WHERE label = 'Swap'"
        ).fetchone()
        tx_count = conn.execute(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            (profile[0],),
        ).fetchone()[0]
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = ?,
                last_processed_tx_count = ?,
                last_processed_input_version = ?
            WHERE id = ?
            """,
            ("2026-03-14T17:35:00Z", tx_count, profile[1], profile[0]),
        )
        conn.commit()
    finally:
        conn.close()


class SwapMatchingCliTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-swap-cli-")
        cls.phoenix_csv = Path(cls._tmp.name) / "phoenix.csv"
        cls.phoenix_csv.write_text(_PHOENIX_CSV, encoding="utf-8")
        cls.liquid_csv = Path(cls._tmp.name) / "liquid.csv"
        cls.liquid_csv.write_text(_LIQUID_CSV, encoding="utf-8")

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _fresh_root(self, prefix):
        tmp = tempfile.TemporaryDirectory(prefix=f"kassiber-{prefix}-")
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name) / "data"

    def test_suggest_bulk_pair_unpair_roundtrip(self):
        data_root = self._fresh_root("flow")
        _bootstrap_profile(data_root, self.phoenix_csv, self.liquid_csv)

        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "transfers.suggest")
        self.assertGreaterEqual(payload["data"]["counts"]["total"], 1)
        candidate = payload["data"]["candidates"][0]
        self.assertIn(candidate["confidence"], ("exact", "strong"))
        self.assertIn("swap_fee_msat", candidate)
        self.assertEqual(candidate["default_policy"], "carrying-value")

        payload, code = _run(
            data_root, "transfers", "bulk-pair",
            "--workspace", "Main", "--profile", "Swap",
            "--confidence", "strong",
            "--asset-pair", "LBTC-BTC",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["summary"]["count"], 0)

        payload, code = _run(
            data_root, "transfers", "bulk-pair",
            "--workspace", "Main", "--profile", "Swap",
            "--confidence", "strong",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "transfers.bulk-pair")
        self.assertGreaterEqual(payload["data"]["summary"]["count"], 1)
        self.assertIn("total_swap_fee_msat", payload["data"]["summary"])

        payload, _ = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(payload["data"]["counts"]["total"], 0)

        payload, _ = _run(
            data_root, "transfers", "list",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(payload["kind"], "transfers.list")
        self.assertEqual(len(payload["data"]), 1)
        pair = payload["data"][0]
        self.assertIsNotNone(pair["swap_fee_msat"])
        self.assertEqual(pair["pair_source"], "bulk_selected")
        self.assertIn(pair["confidence_at_pair"], ("exact", "strong"))

        pair_id = pair["id"]
        payload, _ = _run(
            data_root, "transfers", "unpair",
            "--workspace", "Main", "--profile", "Swap",
            "--pair-id", pair_id,
        )
        self.assertEqual(payload["kind"], "transfers.unpair")

        payload, _ = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertGreaterEqual(payload["data"]["counts"]["total"], 1)

    def test_dismiss_blocks_candidate_until_expiry(self):
        data_root = self._fresh_root("dismiss")
        _bootstrap_profile(data_root, self.phoenix_csv, self.liquid_csv)

        payload, _ = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertGreaterEqual(payload["data"]["counts"]["total"], 1)
        candidate = payload["data"]["candidates"][0]

        payload, code = _run(
            data_root, "transfers", "dismiss",
            "--workspace", "Main", "--profile", "Swap",
            "--tx-out", candidate["out_id"],
            "--tx-in", candidate["in_id"],
            "--reason", "not actually a swap",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "transfers.dismiss")
        self.assertEqual(payload["data"]["reason"], "not actually a swap")
        self.assertIsNotNone(payload["data"]["expires_at"])

        payload, _ = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(payload["data"]["counts"]["total"], 0)

    def test_rules_crud(self):
        data_root = self._fresh_root("rules")
        _run(data_root, "init")
        _run(data_root, "workspaces", "create", "Main")
        _run(
            data_root, "profiles", "create",
            "--workspace", "Main",
            "--tax-country", "at",
            "P",
        )

        payload, code = _run(
            data_root, "transfers", "rules", "create",
            "--workspace", "Main", "--profile", "P",
            "--name", "Phoenix to Liquid",
            "--predicate", json.dumps({"out_wallet_kind": "phoenix"}),
            "--kind", "submarine-swap",
            "--policy", "carrying-value",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "transfers.rules.create")
        rule_id = payload["data"]["id"]
        self.assertTrue(payload["data"]["enabled"])
        self.assertEqual(payload["data"]["predicate"]["out_wallet_kind"], "phoenix")

        payload, _ = _run(
            data_root, "transfers", "rules", "list",
            "--workspace", "Main", "--profile", "P",
        )
        self.assertEqual(payload["kind"], "transfers.rules.list")
        self.assertEqual(len(payload["data"]), 1)

        payload, _ = _run(
            data_root, "transfers", "rules", "disable",
            "--workspace", "Main", "--profile", "P",
            "--rule-id", rule_id,
        )
        self.assertFalse(payload["data"]["enabled"])

        payload, _ = _run(
            data_root, "transfers", "rules", "enable",
            "--workspace", "Main", "--profile", "P",
            "--rule-id", rule_id,
        )
        self.assertTrue(payload["data"]["enabled"])

        payload, _ = _run(
            data_root, "transfers", "rules", "delete",
            "--workspace", "Main", "--profile", "P",
            "--rule-id", rule_id,
        )
        self.assertEqual(payload["data"]["deleted"], rule_id)

    def test_rules_apply_pairs_matching_candidates(self):
        data_root = self._fresh_root("rules-apply")
        _bootstrap_profile(data_root, self.phoenix_csv, self.liquid_csv)

        payload, code = _run(
            data_root, "transfers", "rules", "create",
            "--workspace", "Main", "--profile", "Swap",
            "--name", "Phoenix swap outputs",
            "--predicate", json.dumps({"out_wallet_kind": "phoenix"}),
            "--kind", "submarine-swap",
            "--policy", "carrying-value",
        )
        self.assertEqual(code, 0, payload)

        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(code, 0, payload)
        candidate = payload["data"]["candidates"][0]
        self.assertEqual(candidate["rule_match"]["rule_name"], "Phoenix swap outputs")
        self.assertEqual(payload["data"]["counts"]["rule_matches"], 1)

        payload, code = _run(
            data_root, "transfers", "rules", "apply",
            "--workspace", "Main", "--profile", "Swap",
            "--asset-pair", "LBTC-BTC",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["summary"]["count"], 0)
        payload, _ = _run(
            data_root, "transfers", "list",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(payload["data"], [])

        payload, code = _run(
            data_root, "transfers", "rules", "apply",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "transfers.rules.apply")
        self.assertEqual(payload["data"]["summary"]["count"], 1)

        payload, _ = _run(
            data_root, "transfers", "list",
            "--workspace", "Main", "--profile", "Swap",
        )
        pair = payload["data"][0]
        self.assertEqual(pair["pair_source"], "rule_auto")
        self.assertEqual(pair["confidence_at_pair"], "strong")

    def test_tax_summary_csv_surfaces_swap_fee_columns(self):
        data_root = self._fresh_root("tax-summary-fees")
        _bootstrap_profile(data_root, self.phoenix_csv, self.liquid_csv)
        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(code, 0, payload)
        candidate = payload["data"]["candidates"][0]
        payload, code = _run(
            data_root, "transfers", "pair",
            "--workspace", "Main", "--profile", "Swap",
            "--tx-out", candidate["out_id"],
            "--tx-in", candidate["in_id"],
            "--kind", "submarine-swap",
            "--policy", "taxable",
        )
        self.assertEqual(code, 0, payload)
        _mark_journals_processed(data_root)

        result = _run_raw(
            data_root,
            "--format", "csv",
            "reports", "tax-summary",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        header = result.stdout.splitlines()[0]
        self.assertIn("total_swap_fee_msat", header)
        self.assertIn("swap_fees_total", result.stdout)

    def test_views_crud(self):
        data_root = self._fresh_root("views")
        _run(data_root, "init")
        _run(data_root, "workspaces", "create", "Main")
        _run(
            data_root, "profiles", "create",
            "--workspace", "Main",
            "--tax-country", "at",
            "P",
        )

        payload, code = _run(
            data_root, "views", "create",
            "--workspace", "Main", "--profile", "P",
            "--surface", "swap_candidates",
            "--name", "Boltz pegouts",
            "--filter", json.dumps({"asset_pair": "LBTC-BTC", "min_confidence": "strong"}),
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "views.create")
        view_id = payload["data"]["id"]
        self.assertEqual(payload["data"]["filter"]["asset_pair"], "LBTC-BTC")

        payload, _ = _run(
            data_root, "views", "list",
            "--workspace", "Main", "--profile", "P",
            "--surface", "swap_candidates",
        )
        self.assertEqual(payload["kind"], "views.list")
        self.assertEqual(len(payload["data"]), 1)

        payload, _ = _run(
            data_root, "views", "delete",
            "--workspace", "Main", "--profile", "P",
            "--view-id", view_id,
        )
        self.assertEqual(payload["data"]["deleted"], view_id)


if __name__ == "__main__":
    unittest.main()
