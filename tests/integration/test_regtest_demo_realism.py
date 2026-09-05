from __future__ import annotations

from decimal import Decimal
from pathlib import Path
import unittest
import sqlite3
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


    @staticmethod
    def _native_graph(*, foreign: bool = True):
        def output(value, script, n=0):
            return {"value": value, "scriptPubKey": {"hex": script}, "n": n}
        previous = {("a", 0): output("0.2", "own-a"),
                    ("b", 0): output("0.3", "own-b")}
        outputs = [output("0.15", "own-a", 0), output("0.25", "own-b", 1),
                   output("0.05", "own-c", 2), output("0.05", "own-d", 3)]
        if foreign:
            previous[("foreign", 0)] = output("0.4", "foreign")
            outputs.append(output("0.39998", "foreign", 4))
        else:
            outputs[0]["value"] = "0.14998"
        tx = {"txid": "joint", "vin": [{"txid": txid, "vout": vout} for txid, vout in previous],
              "vout": outputs}
        return tx, previous, {"own-" + key: key for key in "abcd"}

    def test_native_oracle_keeps_foreign_fee_out_of_own_conserving_graph(self) -> None:
        event = regtest_demo._native_event_from_graph(*self._native_graph())
        self.assertEqual(event["wallet_deltas_msat"],
                         {"a": -5_000_000_000, "b": -5_000_000_000,
                          "c": 5_000_000_000, "d": 5_000_000_000})
        self.assertEqual(event["network_fee_msat"], 2_000_000)
        self.assertEqual(event["own_fee_msat"], 0)
        self.assertEqual(event["foreign_input_count"], 1)

    def test_native_oracle_counts_all_owned_fee_once_and_rejects_unknown_split(self) -> None:
        event = regtest_demo._native_event_from_graph(*self._native_graph(foreign=False))
        self.assertEqual(event["own_fee_msat"], 2_000_000)
        tx, previous, scripts = self._native_graph()
        tx["vout"][0]["value"] = "0.14999"
        with self.assertRaisesRegex(RuntimeError, "unexplained own principal"):
            regtest_demo._native_event_from_graph(tx, previous, scripts)

    @staticmethod
    def _native_wallets():
        return {key: regtest_demo.DemoWallet(
            key=key, label=key, account="asset", core_wallet="core-" + key,
            address="address-" + key, addresses=["address-" + key], kassiber_id=key,
        ) for key in "abcd"}

    def test_coinjoin_builder_conserves_own_inputs_and_uses_foreign_signer(self) -> None:
        wallets = self._native_wallets()
        operation = {"signers": ["a", "b"], "outputs": [{"to": "c"}, {"to": "d"}],
                     "equal_output_btc": "0.05", "fee_btc": "0.00002"}
        inputs = [{"txid": key, "vout": 0, "amount": amount}
                  for key, amount in [("a", "0.2"), ("b", "0.3"), ("foreign", "0.4")]]
        foreign_utxo = {**inputs[-1], "confirmations": 2, "spendable": True}
        with patch.object(regtest_demo, "_select_one_utxo", side_effect=inputs[:2]), patch.object(
            regtest_demo, "rpc", return_value=[foreign_utxo]
        ), patch.object(regtest_demo, "_send_raw_transaction", return_value="joint") as send:
            regtest_demo._send_coinjoin_shape("url", "user", "pass", wallets, operation,
                                              "foreign-address", "foreign-core")
        outputs, signers = send.call_args.args[4:6]
        self.assertEqual(signers, ["core-a", "core-b", "foreign-core"])
        self.assertEqual(sum(value for key, value in outputs.items() if key != "foreign-address"),
                         Decimal("0.5"))
        self.assertEqual(outputs["foreign-address"], Decimal("0.39998"))
        self.assertEqual(Decimal("0.9") - sum(outputs.values()), Decimal("0.00002"))

    def test_coinjoin_foreign_payer_spends_confirmed_wallet_change_outside_initial_address(self) -> None:
        wallets = self._native_wallets()
        operation = {"signers": ["a", "b"], "outputs": [{"to": "c"}, {"to": "d"}],
                     "equal_output_btc": "0.05", "fee_btc": "0.00002"}
        def listunspent(_url, _username, _password, method, params, *, wallet):
            self.assertEqual(method, "listunspent")
            if wallet != "foreign-core":
                return [{"txid": wallet, "vout": 0, "amount": "0.2", "spendable": True}]
            # Core sendmany consumed the original funded address and generated
            # wallet change. An initial-address filter therefore returns nothing.
            if len(params) > 2 and params[2]:
                return []
            return [
                {"txid": "unconfirmed", "vout": 0, "amount": "2", "confirmations": 0, "spendable": True},
                {"txid": "unspendable", "vout": 0, "amount": "1", "confirmations": 4, "spendable": False},
                {"txid": "foreign-change", "vout": 1, "amount": "0.4", "confirmations": 3,
                 "spendable": True, "address": "new-foreign-change-address"},
            ]
        with patch.object(regtest_demo, "rpc", side_effect=listunspent) as rpc, patch.object(
            regtest_demo, "_send_raw_transaction", return_value="joint"
        ) as send:
            regtest_demo._send_coinjoin_shape("url", "user", "pass", wallets, operation,
                                              "foreign-initial-address", "foreign-core")
        self.assertEqual(send.call_args.args[3][-1], {"txid": "foreign-change", "vout": 1})
        self.assertEqual(send.call_args.args[4]["foreign-initial-address"], Decimal("0.39998"))
        self.assertEqual(rpc.call_args.args[4][:3], [1, 9999999, []])

    def test_payjoin_builder_and_truth_use_net_merchant_receipt(self) -> None:
        wallets = self._native_wallets()
        operation = {"id": "payjoin", "kind": "payjoin_shape", "payer": "a", "merchant": "b",
                     "payment_btc": "0.0008", "fee_btc": "0.00002310"}
        inputs = [{"txid": "a", "vout": 0, "amount": "0.1"},
                  {"txid": "b", "vout": 0, "amount": "0.02"}]
        truth = regtest_demo.DemoTruth("native")
        with patch.object(regtest_demo, "_select_one_utxo", side_effect=inputs), patch.object(
            regtest_demo, "_send_raw_transaction", return_value="payjoin"
        ) as send:
            regtest_demo._execute_scenario_operation(
                "url", "user", "pass", wallets, operation,
                counterparty_wallets={"customer_pool": "foreign-core"},
                counterparty_addresses={"supplier": "foreign-address"}, txids={}, truth=truth,
            )
        outputs = send.call_args.args[4]
        self.assertEqual(outputs["address-b"] - Decimal("0.02"), Decimal("0.0008"))
        self.assertEqual(Decimal("0.1") - outputs["address-a"], Decimal("0.00082310"))
        self.assertEqual({row["wallet_key"]: row["direction"] for row in truth.transaction_rows},
                         {"a": "outbound", "b": "inbound"})

    def test_custody_oracle_fails_on_missing_authored_or_fee_inflated_decisions(self) -> None:
        # Minimal persistence contract: the independent graph oracle must reject
        # incorrect stored accounting even if aggregate report counts look fine.
        for failure in (None, "missing", "authored", "fee"):
            with self.subTest(failure=failure):
                conn = sqlite3.connect(":memory:")
                conn.row_factory = sqlite3.Row
                conn.executescript("""
                    CREATE TABLE transactions(id TEXT, external_id TEXT);
                    CREATE TABLE journal_custody_decisions(
                        source_wallet_id TEXT,target_wallet_id TEXT,source_start_msat INTEGER,
                        source_end_msat INTEGER,state TEXT,component_id TEXT,
                        source_transaction_id TEXT,target_transaction_id TEXT,
                        source_asset TEXT,target_asset TEXT);
                    CREATE TABLE journal_entries(transaction_id TEXT,entry_type TEXT,quantity INTEGER,asset TEXT);
                    INSERT INTO transactions VALUES ('out','joint'),('in','joint');
                """)
                if failure != "missing":
                    conn.execute("INSERT INTO journal_custody_decisions VALUES(?,?,?,?,?,?,?,?,?,?)",
                                 ("a", "b", 0, 100, "internal_verified",
                                  "manual-review" if failure == "authored" else None,
                                  "out", "in", "BTC", "BTC"))
                conn.execute("INSERT INTO journal_entries VALUES('out','transfer_fee',?,'BTC')",
                             (-3 if failure == "fee" else -2,))
                truth = regtest_demo.DemoTruth("native")
                truth.native_transfers = [{"txid": "joint", "operation_ids": ["native"],
                    "wallet_deltas_msat": {"a": -102, "b": 100},
                    "own_fee_msat": 2, "network_fee_msat": 2, "foreign_input_count": 0}]
                with patch.object(regtest_demo, "open_db", return_value=conn):
                    if failure is None:
                        result = regtest_demo._assert_native_transfer_truth(Path("/unused"), truth, require_fees=True)
                        self.assertEqual(result["own_fees_msat"], 2)
                    else:
                        with self.assertRaises(RuntimeError):
                            regtest_demo._assert_native_transfer_truth(Path("/unused"), truth, require_fees=True)
        with self.assertRaisesRegex(RuntimeError, "truth is empty"):
            regtest_demo._assert_native_transfer_truth(Path("/unused"), regtest_demo.DemoTruth("empty"), require_fees=True)

    def test_native_operations_cannot_gain_authored_transfer_pairs(self) -> None:
        txids = {f"{bridge['id']}_{leg}": f"{bridge['id']}-{leg}"
                 for bridge in self.scenario["stress"]["swap_bridges"] for leg in ["out", "in"]}
        truth = regtest_demo.DemoTruth("native")
        with patch.object(regtest_demo, "run_cli", return_value={"data": {}}) as cli:
            pairs = regtest_demo._pair_transfers(Path("/unused"), self.scenario, txids, truth)
        self.assertEqual(len(pairs), 3)
        self.assertEqual(cli.call_count, 3)
        self.assertTrue(all(row["out_external_id"] != row["in_external_id"] for row in truth.transfer_pairs))


if __name__ == "__main__":
    unittest.main()
