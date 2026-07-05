from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class LightningBusinessPlanTest(unittest.TestCase):
    def _plan(self, *args: str) -> dict:
        script = ROOT / "dev" / "regtest" / "lightning-business-plan.py"
        return json.loads(
            subprocess.check_output([sys.executable, str(script), *args], cwd=ROOT, text=True)
        )

    def _sum_msat(self, plan: dict, key: str) -> int:
        return sum(int(row["amount_msat"]) for row in plan["lightning"][key])

    def test_plan_is_seeded_and_cross_layer(self):
        cmd = [
            "--seed",
            "unit-seed",
            "--capacity-multiplier",
            "0.35",
            "--channel-capacity-sat",
            "5000000",
        ]

        first = self._plan(*cmd)
        second = self._plan(*cmd)

        self.assertEqual(first, second)
        self.assertEqual(first["traffic_model"]["inspired_by"], "bitcoin-dev-project/sim-ln")
        self.assertEqual(first["traffic_model"]["mode"], "seeded-defined-activity")
        self.assertGreater(first["traffic_model"]["turnover_target_msat"], 0)
        self.assertRegex(first["traffic_model"]["plan_hash"], r"^[0-9a-f]{64}$")
        self.assertGreaterEqual(len(first["lightning"]["merchant_invoices"]), 5)
        self.assertGreaterEqual(len(first["lightning"]["supplier_invoices"]), 2)
        self.assertGreaterEqual(len(first["lightning"]["routed_customer_supplier"]), 3)
        self.assertEqual(len(first["lightning"]["expired_invoices"]), 1)
        self.assertEqual(len(first["lightning"]["failed_payments"]), 1)
        self.assertEqual(len(first["lightning"]["lnd_invoices"]), 2)
        self.assertEqual(len(first["lightning"]["lnd_payments"]), 2)
        self.assertGreater(
            first["lightning"]["failed_payments"][0]["amount_msat"],
            5_000_000_000,
        )
        self.assertGreaterEqual(len(first["mainchain"]["topups"]), 3)
        self.assertGreaterEqual(len(first["mainchain"]["withdrawals"]), 2)
        self.assertIn("kassiber-ln-customer-l1", first["mainchain"]["actor_wallets"])

        high = self._plan(
            "--seed",
            "unit-seed",
            "--capacity-multiplier",
            "0.70",
            "--channel-capacity-sat",
            "5000000",
        )
        self.assertGreater(
            high["traffic_model"]["turnover_target_msat"],
            first["traffic_model"]["turnover_target_msat"],
        )
        self.assertNotEqual(
            high["traffic_model"]["plan_hash"],
            first["traffic_model"]["plan_hash"],
        )
        self.assertTrue(high["traffic_model"]["liquidity_capped"])
        self.assertLessEqual(
            self._sum_msat(high, "merchant_invoices")
            + self._sum_msat(high, "routed_customer_supplier"),
            high["traffic_model"]["liquidity_budget_msat"],
        )
        self.assertNotEqual(
            len(high["lightning"]["merchant_invoices"]),
            len(first["lightning"]["merchant_invoices"]),
        )

    def test_seed_strings_are_opaque(self):
        one = self._plan("--seed", "1")
        padded = self._plan("--seed", "001")

        self.assertNotEqual(one["traffic_model"]["plan_hash"], padded["traffic_model"]["plan_hash"])
        self.assertNotEqual(one["lightning"], padded["lightning"])

    def test_scenario_retries_transient_invoice_reads(self):
        script = (ROOT / "dev" / "regtest" / "lightning-business-scenario.sh").read_text(
            encoding="utf-8"
        )

        self.assertIn("invoice_status_once()", script)
        self.assertIn("invoice_matches_plan_once()", script)
        self.assertIn("ensure_lnd_invoice_paid_by_cln()", script)
        self.assertIn("ensure_lnd_paid_cln_invoice()", script)
        self.assertIn("for attempt in $(seq 1 20)", script)
        self.assertIn("except ValueError:", script)
        self.assertNotIn('python3 - "$expected_amount_msat" "$expected_description"', script)

    def test_seed_updates_stale_demo_lightning_cli_wrapper(self):
        script = (ROOT / "tests" / "integration" / "lightning_business_regtest.py").read_text(
            encoding="utf-8"
        )

        self.assertIn("KASSIBER_LIGHTNING_BUSINESS_MERCHANT_CLI", script)
        self.assertIn('"backends"', script)
        self.assertIn('"update"', script)
        self.assertIn('"--lightning-cli"', script)
        self.assertIn("if _book_exists(data_root):", script)
        self.assertIn("_ensure_backend(data_root, merchant_cli)", script)
        self.assertIn("_ensure_backup_lnd_source(data_root)", script)
        self.assertIn("BACKUP_LND_MACAROON_HEX", script)
        self.assertIn("lnd_snapshot", script)


if __name__ == "__main__":
    unittest.main()
