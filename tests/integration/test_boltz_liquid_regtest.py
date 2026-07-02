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


if __name__ == "__main__":
    unittest.main()
