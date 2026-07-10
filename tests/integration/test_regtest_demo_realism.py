from __future__ import annotations

from decimal import Decimal
from pathlib import Path
import unittest

from tests.integration import regtest_demo


class RegtestDemoRealismTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.scenario = regtest_demo.load_scenario(
            Path("dev/regtest/scenarios/full_accounting.json")
        )

    def test_fiat_recurring_amount_tracks_historical_kraken_rate(self) -> None:
        history = regtest_demo._load_bundled_daily_rates("BTC-EUR")
        early_ts = regtest_demo._parse_iso_to_ts("2019-01-18T12:00:00Z")
        late_ts = regtest_demo._parse_iso_to_ts("2025-11-01T12:00:00Z")
        early = regtest_demo._recurring_btc(
            {"amount_eur": "8000.00"},
            rate=regtest_demo._cached_rate_at_or_before(history, early_ts),
            label="payroll",
        )
        late = regtest_demo._recurring_btc(
            {"amount_eur": "8000.00"},
            rate=regtest_demo._cached_rate_at_or_before(history, late_ts),
            label="payroll",
        )

        self.assertGreater(early / late, Decimal("25"))
        self.assertEqual(
            (early * regtest_demo._cached_rate_at_or_before(history, early_ts)).quantize(Decimal("0.01")),
            Decimal("8000.00"),
        )
        self.assertEqual(
            (late * regtest_demo._cached_rate_at_or_before(history, late_ts)).quantize(Decimal("0.01")),
            Decimal("8000.00"),
        )

    def test_cycle_clock_and_activity_modes_are_deterministic(self) -> None:
        cycles = range(1, int(self.scenario["stress"]["cycles"]) + 1)
        skipped = [cycle for cycle in cycles if regtest_demo._cycle_activity_mode(cycle) == "skip"]
        doubled = [cycle for cycle in cycles if regtest_demo._cycle_activity_mode(cycle) == "double"]

        self.assertEqual(skipped, [46, 56])
        self.assertEqual(doubled, [26, 36, 45, 55, 81])
        self.assertEqual(regtest_demo._cycle_activity_mode(1), "normal")

        first_target = regtest_demo._parse_iso_to_ts(self.scenario["base_time"])
        timestamps = [
            regtest_demo._cycle_timestamp(first_target, cycle, 30)
            for cycle in range(1, 85)
        ]
        self.assertEqual(timestamps, sorted(timestamps))
        self.assertGreater(len({timestamp % regtest_demo.SECONDS_PER_DAY for timestamp in timestamps}), 12)

    def test_manifest_covers_realism_contract(self) -> None:
        stress = self.scenario["stress"]
        scheduled = [operation for operation in self.scenario["operations"] if operation.get("cycle")]
        self.assertTrue(scheduled)
        self.assertTrue(all("amount_eur" in value for value in stress["receipt_btc"].values()))
        self.assertTrue(all("amount_eur" in value for value in stress["payment_btc"].values()))
        self.assertTrue(all("amount_eur" in expense for expense in stress["business_expenses"]["schedule"]))
        self.assertEqual(self.scenario["expected"]["open_collateral_locks"], 1)
        self.assertEqual(len(stress["fee_curve"]), 5)
        self.assertTrue(stress["pool_payouts"]["enabled"])

        script_types = {
            wallet.get("address_type")
            for wallet in self.scenario["wallets"]
            if wallet.get("address_type")
        }
        self.assertTrue({"legacy", "p2sh-segwit", "bech32", "bech32m"}.issubset(script_types))
        self.assertGreaterEqual(
            min(
                int(wallet["addresses"])
                for wallet in self.scenario["wallets"]
                if wallet.get("address_type")
            ),
            8,
        )

    def test_fee_curve_changes_by_era(self) -> None:
        stress = self.scenario["stress"]
        multiplier = lambda value: regtest_demo._fee_curve_multiplier(
            stress, regtest_demo._parse_iso_to_ts(value)
        )
        self.assertEqual(multiplier("2019-06-01T00:00:00Z"), Decimal("0.55"))
        self.assertEqual(multiplier("2021-06-01T00:00:00Z"), Decimal("2.40"))
        self.assertEqual(multiplier("2023-06-01T00:00:00Z"), Decimal("3.20"))
        self.assertEqual(multiplier("2025-06-01T00:00:00Z"), Decimal("1.35"))


if __name__ == "__main__":
    unittest.main()
