from __future__ import annotations

from decimal import Decimal
from pathlib import Path
import unittest
from unittest.mock import patch

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
        self.assertEqual(len(stress["internal_transfers"]), 4)
        self.assertTrue(all(rotation["mode"] == "sweep" for rotation in stress["wallet_rotations"]))
        self.assertEqual(len(stress["liquid_wallet_rotations"]), 1)
        consolidations = [
            operation
            for operation in self.scenario["operations"]
            if operation["kind"] == "many_input_consolidation"
        ]
        self.assertEqual(len(consolidations), 3)
        self.assertTrue(all(operation["fee_curve"] for operation in consolidations))
        self.assertEqual(self.scenario["expected"]["deprecated_wallet_max_utxos"], 1)

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

    def test_internal_transfer_policy_uses_fiat_thresholds(self) -> None:
        sweep = {
            "kind": "sweep_excess",
            "source_reserve_eur": "20000",
            "min_amount_eur": "1000",
            "fraction": "0.50",
        }
        self.assertEqual(
            regtest_demo._internal_transfer_amount(
                sweep,
                rate=Decimal("40000"),
                source_balance=Decimal("2"),
                target_balance=Decimal("0"),
                fee=Decimal("0.00001"),
            ),
            Decimal("0.74999500"),
        )
        refill = {
            "kind": "refill_to_target",
            "source_reserve_eur": "20000",
            "target_balance_eur": "18000",
            "min_amount_eur": "1000",
        }
        self.assertEqual(
            regtest_demo._internal_transfer_amount(
                refill,
                rate=Decimal("40000"),
                source_balance=Decimal("2"),
                target_balance=Decimal("0.20"),
                fee=Decimal("0.00001"),
            ),
            Decimal("0.25000000"),
        )

    def test_core_rotation_sweeps_every_utxo_without_change(self) -> None:
        sender = regtest_demo.DemoWallet(
            key="old",
            label="Old",
            account="treasury",
            core_wallet="old-core",
            address="old-address",
            addresses=["old-address"],
        )
        receiver = regtest_demo.DemoWallet(
            key="new",
            label="New",
            account="treasury",
            core_wallet="new-core",
            address="new-address",
            addresses=["new-address"],
        )
        utxos = [
            {"txid": "a" * 64, "vout": 0, "amount": "1.25"},
            {"txid": "b" * 64, "vout": 1, "amount": "0.75"},
        ]
        with patch.object(regtest_demo, "_wallet_utxos", return_value=utxos), patch.object(
            regtest_demo,
            "_send_raw_transaction",
            return_value="c" * 64,
        ) as send:
            txid = regtest_demo._sweep_core_wallet(
                "http://127.0.0.1",
                "user",
                "pass",
                sender,
                receiver,
                Decimal("0.00001000"),
            )

        self.assertEqual(txid, "c" * 64)
        self.assertEqual(
            send.call_args.args[4],
            {"new-address": Decimal("1.99999000")},
        )
        self.assertEqual(len(send.call_args.args[3]), 2)

    def test_core_rotation_keeps_configured_residual_as_an_output(self) -> None:
        sender = regtest_demo.DemoWallet(
            key="old",
            label="Old",
            account="merchant",
            core_wallet="old-core",
            address="old-address",
            addresses=["old-address"],
        )
        receiver = regtest_demo.DemoWallet(
            key="new",
            label="New",
            account="merchant",
            core_wallet="new-core",
            address="new-address",
            addresses=["new-address"],
        )
        with patch.object(
            regtest_demo,
            "_wallet_utxos",
            return_value=[{"txid": "a" * 64, "vout": 0, "amount": "1.00"}],
        ), patch.object(regtest_demo, "_send_raw_transaction", return_value="b" * 64) as send:
            regtest_demo._sweep_core_wallet(
                "http://127.0.0.1",
                "user",
                "pass",
                sender,
                receiver,
                Decimal("0.00001000"),
                residual=Decimal("0.00080000"),
            )

        self.assertEqual(
            send.call_args.args[4],
            {
                "new-address": Decimal("0.99919000"),
                "old-address": Decimal("0.00080000"),
            },
        )


if __name__ == "__main__":
    unittest.main()
