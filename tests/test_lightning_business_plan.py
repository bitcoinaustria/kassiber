from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


class LightningBusinessPlanTest(unittest.TestCase):
    def test_plan_is_seeded_and_cross_layer(self):
        script = ROOT / "dev" / "regtest" / "lightning-business-plan.py"
        cmd = [
            sys.executable,
            str(script),
            "--seed",
            "unit-seed",
            "--capacity-multiplier",
            "0.35",
            "--channel-capacity-sat",
            "5000000",
        ]

        first = json.loads(subprocess.check_output(cmd, cwd=ROOT, text=True))
        second = json.loads(subprocess.check_output(cmd, cwd=ROOT, text=True))

        self.assertEqual(first, second)
        self.assertEqual(first["traffic_model"]["inspired_by"], "bitcoin-dev-project/sim-ln")
        self.assertEqual(first["traffic_model"]["mode"], "seeded-defined-activity")
        self.assertGreater(first["traffic_model"]["turnover_target_msat"], 0)
        self.assertGreaterEqual(len(first["lightning"]["merchant_invoices"]), 5)
        self.assertGreaterEqual(len(first["lightning"]["supplier_invoices"]), 2)
        self.assertGreaterEqual(len(first["lightning"]["routed_customer_supplier"]), 3)
        self.assertEqual(len(first["lightning"]["expired_invoices"]), 1)
        self.assertEqual(len(first["lightning"]["failed_payments"]), 1)
        self.assertGreater(
            first["lightning"]["failed_payments"][0]["amount_msat"],
            5_000_000_000,
        )
        self.assertGreaterEqual(len(first["mainchain"]["topups"]), 3)
        self.assertGreaterEqual(len(first["mainchain"]["withdrawals"]), 2)
        self.assertIn("kassiber-ln-customer-l1", first["mainchain"]["actor_wallets"])

        high = json.loads(
            subprocess.check_output(
                [
                    sys.executable,
                    str(script),
                    "--seed",
                    "unit-seed",
                    "--capacity-multiplier",
                    "0.70",
                    "--channel-capacity-sat",
                    "5000000",
                ],
                cwd=ROOT,
                text=True,
            )
        )
        self.assertGreater(
            high["traffic_model"]["turnover_target_msat"],
            first["traffic_model"]["turnover_target_msat"],
        )


if __name__ == "__main__":
    unittest.main()
