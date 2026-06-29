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

_COLD_TO_HOT_OUT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-15T10:00:00Z,cold-hot-txid,outbound,BTC,0.10000000,0.00001000,40000,Move monthly spend to hot wallet
"""

_COLD_TO_HOT_IN_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-15T10:05:00Z,cold-hot-txid,inbound,BTC,0.09999000,0,40000,Receive monthly spend from cold wallet
"""

_BTC_TRANSFER_OUT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-16T10:00:00Z,btc-transfer-out,outbound,BTC,0.10000000,0.00001000,40000,Move to hot wallet
"""

_BTC_TRANSFER_IN_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-16T10:05:00Z,btc-transfer-in,inbound,BTC,0.09999000,0,40000,Receive from cold wallet
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

    def test_same_txid_self_transfer_not_suggested_as_swap(self):
        data_root = self._fresh_root("same-txid-transfer")
        out_csv = Path(self._tmp.name) / "cold-to-hot-out.csv"
        in_csv = Path(self._tmp.name) / "cold-to-hot-in.csv"
        out_csv.write_text(_COLD_TO_HOT_OUT_CSV, encoding="utf-8")
        in_csv.write_text(_COLD_TO_HOT_IN_CSV, encoding="utf-8")

        _run(data_root, "init")
        _run(data_root, "workspaces", "create", "Main")
        _run(
            data_root, "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Swap",
        )
        for wallet in ("cold-onchain", "hot-onchain"):
            _run(
                data_root, "wallets", "create",
                "--workspace", "Main",
                "--profile", "Swap",
                "--label", wallet,
                "--kind", "custom",
            )
        _run(
            data_root, "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Swap",
            "--wallet", "cold-onchain",
            "--file", str(out_csv),
        )
        _run(
            data_root, "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Swap",
            "--wallet", "hot-onchain",
            "--file", str(in_csv),
        )

        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "transfers.suggest")
        self.assertEqual(payload["data"]["counts"]["total"], 0)

    def test_candidate_type_splits_same_asset_transfers_from_swaps(self):
        data_root = self._fresh_root("candidate-type")
        out_csv = Path(self._tmp.name) / "btc-transfer-out.csv"
        in_csv = Path(self._tmp.name) / "btc-transfer-in.csv"
        out_csv.write_text(_BTC_TRANSFER_OUT_CSV, encoding="utf-8")
        in_csv.write_text(_BTC_TRANSFER_IN_CSV, encoding="utf-8")

        _run(data_root, "init")
        _run(data_root, "workspaces", "create", "Main")
        _run(
            data_root, "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "at",
            "Swap",
        )
        for wallet in ("cold-onchain", "hot-onchain"):
            _run(
                data_root, "wallets", "create",
                "--workspace", "Main",
                "--profile", "Swap",
                "--label", wallet,
                "--kind", "custom",
            )
        _run(
            data_root, "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Swap",
            "--wallet", "cold-onchain",
            "--file", str(out_csv),
        )
        _run(
            data_root, "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Swap",
            "--wallet", "hot-onchain",
            "--file", str(in_csv),
        )

        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "swap",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["counts"]["total"], 0)

        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "transfer",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["counts"]["total"], 1)
        candidate = payload["data"]["candidates"][0]
        self.assertEqual(candidate["out_asset"], "BTC")
        self.assertEqual(candidate["in_asset"], "BTC")
        self.assertEqual(candidate["candidate_type"], "transfer")
        self.assertEqual(candidate["default_kind"], "manual")
        self.assertEqual(candidate["default_policy"], "carrying-value")

        payload, code = _run(
            data_root, "transfers", "bulk-pair",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "transfer",
            "--confidence", "strong",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["summary"]["count"], 1)

    def test_conflicted_layer_transition_stays_in_transfer_review(self):
        # One outbound BTC leg matches both a same-asset inbound and a BTC->LBTC
        # layer transition. Both are transfer-like now, but the matcher-stamped
        # conflict_size must still keep bulk-pair from silently choosing either
        # interpretation.
        data_root = self._fresh_root("split-conflict")
        out_csv = Path(self._tmp.name) / "split-out.csv"
        in_btc_csv = Path(self._tmp.name) / "split-in-btc.csv"
        in_lbtc_csv = Path(self._tmp.name) / "split-in-lbtc.csv"
        out_csv.write_text(_BTC_TRANSFER_OUT_CSV, encoding="utf-8")
        in_btc_csv.write_text(_BTC_TRANSFER_IN_CSV, encoding="utf-8")
        in_lbtc_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-16T10:10:00Z,lbtc-swap-in,inbound,LBTC,0.09998000,0,40000,Possible peg-in\n",
            encoding="utf-8",
        )

        _run(data_root, "init")
        _run(data_root, "workspaces", "create", "Main")
        _run(
            data_root, "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "at",
            "Swap",
        )
        for wallet, kind, csv_path in (
            ("cold-onchain", "wasabi", out_csv),
            ("hot-onchain", "custom", in_btc_csv),
            ("liquid-vault", "wasabi", in_lbtc_csv),
        ):
            _run(
                data_root, "wallets", "create",
                "--workspace", "Main",
                "--profile", "Swap",
                "--label", wallet,
                "--kind", kind,
            )
            _run(
                data_root, "wallets", "import-csv",
                "--workspace", "Main",
                "--profile", "Swap",
                "--wallet", wallet,
                "--file", str(csv_path),
            )

        # The transfer view shows both transfer-like interpretations and keeps
        # the shared conflict cluster.
        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "transfer",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["counts"]["total"], 2)
        self.assertEqual(payload["data"]["counts"]["conflicts"], 1)
        candidates = payload["data"]["candidates"]
        self.assertEqual(
            {
                (candidate["out_asset"], candidate["in_asset"])
                for candidate in candidates
            },
            {("BTC", "BTC"), ("BTC", "LBTC")},
        )
        self.assertTrue(
            all(candidate["candidate_type"] == "transfer" for candidate in candidates)
        )
        self.assertTrue(all(candidate["conflict_size"] == 2 for candidate in candidates))

        payload, code = _run(
            data_root, "transfers", "suggest",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "swap",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["counts"]["total"], 0)

        payload, code = _run(
            data_root, "transfers", "bulk-pair",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "transfer",
            "--confidence", "strong",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["summary"]["count"], 0, payload)
        self.assertEqual(payload["data"]["summary"]["skipped_conflicts"], 2, payload)

        payload, code = _run(
            data_root, "transfers", "bulk-pair",
            "--workspace", "Main", "--profile", "Swap",
            "--candidate-type", "swap",
            "--confidence", "strong",
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["summary"]["count"], 0, payload)
        self.assertEqual(payload["data"]["summary"]["skipped_conflicts"], 0, payload)

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
