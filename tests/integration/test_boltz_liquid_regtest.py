from __future__ import annotations

import unittest

from tests.integration import boltz_liquid_regtest
from tests.integration.env import no_egress_guard, skip_unless_env


class BoltzLiquidRegtestTest(unittest.TestCase):
    def test_demo_boltz_bridge_metadata_is_self_contained(self):
        scenario = boltz_liquid_regtest.load_demo_scenario_metadata()
        bridges = boltz_liquid_regtest.boltz_bridge_specs(scenario)

        self.assertEqual(len(bridges), 1)
        self.assertEqual(
            {
                (
                    bridge["boltz_flow"],
                    bridge["boltz_api"],
                    bridge["boltz_from"],
                    bridge["boltz_to"],
                )
                for bridge in bridges
            },
            {("chain-swap", "/v2/swap/chain", "BTC", "L-BTC")},
        )

    @skip_unless_env("KASSIBER_BOLTZ_REGTEST", "local Boltz regtest stack is opt-in")
    def test_live_boltz_liquid_pairs_cover_demo_bridge(self):
        with no_egress_guard(enabled=True):
            probe = boltz_liquid_regtest.probe_boltz_liquid()

        self.assertIn("version", probe)
        self.assertGreaterEqual(int(probe["heights"]["BTC"]), 0)
        self.assertGreaterEqual(int(probe["heights"]["L-BTC"]), 0)
        self.assertIn("hash", probe["pairs"]["bitcoin_to_liquid"])

        covered = boltz_liquid_regtest.verify_demo_boltz_coverage(
            probe,
            boltz_liquid_regtest.load_demo_scenario_metadata(),
        )
        self.assertEqual(
            {(row["flow"], row["from"], row["to"]) for row in covered},
            {("chain-swap", "BTC", "L-BTC")},
        )

    @skip_unless_env("KASSIBER_BOLTZ_REGTEST", "local Boltz regtest stack is opt-in")
    def test_live_boltz_liquid_execution_covers_swap_and_payment_accounting(self):
        with no_egress_guard(enabled=True):
            summary = boltz_liquid_regtest.run_boltz_liquid_scenario()

        payment = summary["executed"]["liquid_payment"]
        swap = summary["executed"]["liquid_submarine_swap"]
        accounting = summary["accounting"]

        self.assertEqual(payment["asset"], "LBTC")
        self.assertRegex(payment["txid"], r"^[0-9a-f]{64}$")
        self.assertEqual(swap["payment_hash"], accounting["swap_lockup"]["payment_hash"])
        self.assertEqual(accounting["swap_lockup"]["payment_hash_source"], "boltz-regtest")
        self.assertEqual(accounting["plain_payment"]["asset"], "LBTC")
        self.assertEqual(accounting["plain_payment"]["direction"], "outbound")
        self.assertFalse(accounting["plain_payment"]["paired"])

        candidate = accounting["candidate"]
        self.assertEqual(candidate["confidence"], "exact")
        self.assertEqual(candidate["method"], "payment_hash")
        self.assertEqual(candidate["out_asset"], "LBTC")
        self.assertEqual(candidate["in_asset"], "BTC")
        self.assertEqual(candidate["out_wallet_kind"], "custom")
        self.assertEqual(candidate["in_wallet_kind"], "lnd")
        self.assertEqual(candidate["default_kind"], "submarine-swap")
        self.assertEqual(candidate["candidate_type"], "transfer")

        pair = accounting["pair"]
        self.assertEqual(pair["kind"], "submarine-swap")
        self.assertEqual(pair["out"]["external_id"], swap["lockup_txid"])
        self.assertEqual(pair["out"]["asset"], "LBTC")
        self.assertEqual(pair["in"]["asset"], "BTC")


if __name__ == "__main__":
    unittest.main()
