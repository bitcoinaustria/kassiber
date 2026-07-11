"""Legacy-holdings overlay: importer, tax-engine gate, and reports.

Overlay (non-Bitcoin) assets are overview-only: they import, they show up in
`reports legacy-holdings`, and they must NEVER reach journals, capital gains,
tax summaries, or exit tax. The Bitcoin legs of overlay trades DO book
normally — that is the point (real execution cost basis for BTC acquired by
selling an altcoin). Fixture asset symbols are deliberately fake ("ALT",
"ALT2") — see the repo convention of keeping real altcoin tickers out of
fixtures.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from kassiber.asset_codes import is_tax_engine_asset
from kassiber.errors import AppError
from kassiber.importers import (
    normalize_generic_ledger_record,
    normalize_legacy_holdings_records,
)
from kassiber.core.exchange_imports import normalize_kraken_records

ROOT = Path(__file__).resolve().parent.parent

_LEGACY_CSV = """Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,Counterparty,Note,Tx-ID
Buy,2020-06-01T10:00:00Z,1000,ALT,500,EUR,,,,CEX,legacy buy,alt-buy-1
Trade,2021-04-01T10:00:00Z,0.5,BTC,900,ALT,,,20000,CEX,alt exit,trade-1
Withdrawal,2021-04-02T10:00:00Z,,,0.5,BTC,0.0001,BTC,,,to cold storage,wd-1
"""

# The same Bitcoin-only economics without the overlay wallet: tax outputs of
# both books must be identical (the regression pin for the exclusion gate).
_BTC_ONLY_CSV = """Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,Counterparty,Note,Tx-ID
Buy,2021-04-01T10:00:00Z,0.5,BTC,20000,EUR,,,,CEX,btc buy,trade-1:in
Withdrawal,2021-04-02T10:00:00Z,,,0.5,BTC,0.0001,BTC,,,to cold storage,wd-1
"""


def _run(data_root, *args, input_text=None):
    cmd = [sys.executable, "-m", "kassiber", "--data-root", str(data_root), "--machine", *args]
    result = subprocess.run(
        cmd, cwd=ROOT, capture_output=True, text=True, input=input_text, check=False
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(f"CLI produced no stdout.\nargs: {args}\nstderr: {result.stderr}")
    return json.loads(stdout), result.returncode


class TaxEngineAssetTest(unittest.TestCase):
    def test_bitcoin_family_and_liquid_ids_are_tax_engine_assets(self):
        self.assertTrue(is_tax_engine_asset("BTC"))
        self.assertTrue(is_tax_engine_asset("LBTC"))
        self.assertTrue(is_tax_engine_asset("a" * 64))
        self.assertTrue(is_tax_engine_asset("0123456789abcdef" * 4))

    def test_overlay_assets_are_not(self):
        self.assertFalse(is_tax_engine_asset("ALT"))
        self.assertFalse(is_tax_engine_asset("USDT"))
        self.assertFalse(is_tax_engine_asset(""))
        self.assertFalse(is_tax_engine_asset(None))


class LegacyHoldingsImporterTest(unittest.TestCase):
    def test_trade_row_splits_into_sell_and_buy_legs(self):
        records = normalize_legacy_holdings_records(
            {
                "Type": "Trade",
                "Date": "2021-04-01T10:00:00Z",
                "Sent Amount": "900",
                "Sent Asset": "ALT",
                "Received Amount": "0.5",
                "Received Asset": "BTC",
                "Fiat Value": "20000",
                "Tx-ID": "trade-1",
            }
        )
        self.assertEqual(len(records), 2)
        out_leg, in_leg = records
        self.assertEqual((out_leg["direction"], out_leg["asset"], out_leg["kind"]), ("outbound", "ALT", "sell"))
        self.assertEqual((in_leg["direction"], in_leg["asset"], in_leg["kind"]), ("inbound", "BTC", "buy"))
        self.assertEqual(out_leg["txid"], "trade-1:out")
        self.assertEqual(in_leg["txid"], "trade-1:in")
        # Both legs carry the trade's execution value in the book currency.
        self.assertEqual(out_leg["fiat_value"], Decimal("20000"))
        self.assertEqual(in_leg["fiat_value"], Decimal("20000"))
        self.assertEqual(in_leg["fiat_rate"], Decimal("20000") / Decimal("0.5"))

    def test_single_leg_altcoin_buy_prices_from_cash_leg(self):
        records = normalize_legacy_holdings_records(
            {
                "Type": "Buy",
                "Date": "2020-06-01T10:00:00Z",
                "Received Amount": "1000",
                "Received Asset": "ALT",
                "Sent Amount": "500",
                "Sent Asset": "EUR",
            }
        )
        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["asset"], "ALT")
        self.assertEqual(record["fiat_currency"], "EUR")
        self.assertEqual(record["fiat_value"], Decimal("500"))
        self.assertEqual(record["pricing_source_kind"], "exchange_execution")

    def test_envelope_rejects_more_than_11_decimals(self):
        with self.assertRaises(AppError) as ctx:
            normalize_legacy_holdings_records(
                {
                    "Type": "Deposit",
                    "Date": "2021-01-01",
                    "Received Amount": "0.123456789012345",
                    "Received Asset": "ALT",
                }
            )
        self.assertIn("11 decimal places", str(ctx.exception))

    def test_envelope_rejects_oversized_quantities(self):
        with self.assertRaises(AppError) as ctx:
            normalize_legacy_holdings_records(
                {
                    "Type": "Deposit",
                    "Date": "2021-01-01",
                    "Received Amount": "100000000",
                    "Received Asset": "ALT",
                }
            )
        self.assertIn("maximum storable quantity", str(ctx.exception))

    def test_trade_fee_must_match_a_leg(self):
        with self.assertRaises(AppError):
            normalize_legacy_holdings_records(
                {
                    "Type": "Trade",
                    "Date": "2021-04-01",
                    "Sent Amount": "900",
                    "Sent Asset": "ALT",
                    "Received Amount": "0.5",
                    "Received Asset": "BTC",
                    "Fee Amount": "1",
                    "Fee Asset": "ALT2",
                }
            )

    def test_same_asset_trade_is_rejected(self):
        with self.assertRaises(AppError):
            normalize_legacy_holdings_records(
                {
                    "Type": "Trade",
                    "Date": "2021-04-01",
                    "Sent Amount": "1",
                    "Sent Asset": "ALT",
                    "Received Amount": "1",
                    "Received Asset": "ALT",
                }
            )

    def test_generic_ledger_rejects_unknown_cash_currency(self):
        # Regression: an unknown symbol used to be silently booked as the
        # "fiat" pricing leg, corrupting the execution price.
        with self.assertRaises(AppError) as ctx:
            normalize_generic_ledger_record(
                {
                    "Type": "Sell",
                    "Date": "2021-04-01",
                    "Sent Amount": "0.5",
                    "Sent Asset": "BTC",
                    "Received Amount": "900",
                    "Received Asset": "ALT",
                }
            )
        self.assertIn("unrecognized cash currency", str(ctx.exception))

    def test_generic_ledger_known_fiat_still_works(self):
        record = normalize_generic_ledger_record(
            {
                "Type": "Sell",
                "Date": "2021-04-01",
                "Sent Amount": "0.5",
                "Sent Asset": "BTC",
                "Received Amount": "20000",
                "Received Asset": "EUR",
            }
        )
        self.assertEqual(record["fiat_currency"], "EUR")
        self.assertEqual(record["asset"], "BTC")

    def test_generic_ledger_still_rejects_two_crypto_legs(self):
        with self.assertRaises(AppError):
            normalize_generic_ledger_record(
                {
                    "Type": "Trade",
                    "Date": "2021-04-01",
                    "Sent Amount": "0.5",
                    "Sent Asset": "BTC",
                    "Received Amount": "0.5",
                    "Received Asset": "LBTC",
                }
            )


class KrakenLegacyPassThroughTest(unittest.TestCase):
    def _ledger(self):
        return {
            "result": {
                "ledger": {
                    "L1": {
                        "asset": "ALT",
                        "type": "deposit",
                        "amount": "100",
                        "fee": "0",
                        "time": 1600000000,
                    },
                    "L2": {
                        "asset": "ZEUR",
                        "type": "deposit",
                        "amount": "500",
                        "fee": "0",
                        "time": 1600000000,
                    },
                    "L3": {
                        "asset": "XXBT",
                        "type": "deposit",
                        "amount": "0.1",
                        "fee": "0",
                        "time": 1600000000,
                    },
                },
                "count": 3,
            }
        }

    def test_default_still_skips_non_btc(self):
        records = normalize_kraken_records(self._ledger())
        self.assertEqual([record["asset"] for record in records], ["BTC"])

    def test_include_legacy_passes_altcoins_but_never_fiat(self):
        records = normalize_kraken_records(self._ledger(), include_legacy=True)
        self.assertEqual(sorted(record["asset"] for record in records), ["ALT", "BTC"])

    def test_include_legacy_skips_envelope_violations_with_note(self):
        notes = []
        ledger = {
            "result": {
                "ledger": {
                    "L9": {
                        "asset": "ALT",
                        "type": "deposit",
                        "amount": "999999999",
                        "fee": "0",
                        "time": 1600000000,
                    }
                },
                "count": 1,
            }
        }
        records = normalize_kraken_records(ledger, include_legacy=True, legacy_notes=notes)
        self.assertEqual(records, [])
        self.assertEqual(len(notes), 1)
        self.assertIn("envelope", notes[0]["reason"])

    def test_include_legacy_emits_unpriced_cross_asset_trade_legs(self):
        ledger = {
            "result": {
                "ledger": {
                    "L4": {
                        "asset": "ALT",
                        "type": "trade",
                        "amount": "-900",
                        "fee": "0",
                        "time": 1600000000,
                        "refid": "T1",
                    },
                    "L5": {
                        "asset": "XXBT",
                        "type": "trade",
                        "amount": "0.5",
                        "fee": "0",
                        "time": 1600000000,
                        "refid": "T1",
                    },
                },
                "count": 2,
            }
        }
        trades = {"result": {"trades": {"T1": {"pair": "ALTXBT", "cost": "0.5", "fee": "0"}}, "count": 1}}
        with self.assertRaises(AppError):
            normalize_kraken_records(ledger, trades)
        records = normalize_kraken_records(ledger, trades, include_legacy=True)
        by_asset = {record["asset"]: record for record in records}
        self.assertEqual(set(by_asset), {"ALT", "BTC"})
        self.assertEqual(by_asset["ALT"]["kind"], "sell")
        self.assertEqual(by_asset["BTC"]["kind"], "buy")
        self.assertIsNone(by_asset["BTC"].get("fiat_value"))


class LegacyHoldingsEndToEndTest(unittest.TestCase):
    """Import an overlay book, then pin the gate and the report surfaces."""

    maxDiff = None

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory()
        tmp = Path(cls._tmp.name)
        cls.data_root = tmp / "state"
        cls.legacy_csv = tmp / "legacy.csv"
        cls.legacy_csv.write_text(_LEGACY_CSV, encoding="utf-8")
        cls.btc_only_csv = tmp / "btc-only.csv"
        cls.btc_only_csv.write_text(_BTC_ONLY_CSV, encoding="utf-8")

        payload, code = _run(cls.data_root, "init")
        assert code == 0, payload
        for args in (
            ("workspaces", "create", "T"),
            ("profiles", "create", "--workspace", "T", "--fiat-currency", "EUR", "--tax-country", "at", "Overlay"),
            ("profiles", "create", "--workspace", "T", "--fiat-currency", "EUR", "--tax-country", "at", "Control"),
            (
                "wallets", "create", "--workspace", "T", "--profile", "Overlay",
                "--kind", "legacy-holdings", "--label", "CEX Legacy",
                "--source-file", str(cls.legacy_csv), "--source-format", "legacy_holdings",
            ),
            (
                "wallets", "create", "--workspace", "T", "--profile", "Control",
                "--kind", "custom", "--label", "CEX Legacy",
                "--source-file", str(cls.btc_only_csv), "--source-format", "csv",
            ),
            (
                "wallets", "import-legacy-holdings", "--workspace", "T", "--profile", "Overlay",
                "--wallet", "CEX Legacy", "--file", str(cls.legacy_csv),
            ),
            (
                "wallets", "import-ledger", "--workspace", "T", "--profile", "Control",
                "--wallet", "CEX Legacy", "--file", str(cls.btc_only_csv),
            ),
        ):
            payload, code = _run(cls.data_root, *args)
            assert code == 0, (args, payload)

        cls.process_payloads = {}
        for profile in ("Overlay", "Control"):
            payload, code = _run(cls.data_root, "journals", "process", "--workspace", "T", "--profile", profile)
            assert code == 0, payload
            cls.process_payloads[profile] = payload["data"]

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _report(self, profile, *args):
        payload, code = _run(self.data_root, "reports", *args, "--workspace", "T", "--profile", profile)
        self.assertEqual(code, 0, payload)
        return payload["data"]

    def test_journals_process_warns_about_overlay_exclusion(self):
        warnings = self.process_payloads["Overlay"].get("warnings") or []
        codes = [warning.get("code") for warning in warnings]
        self.assertIn("legacy_holdings_excluded", codes)
        warning = next(w for w in warnings if w.get("code") == "legacy_holdings_excluded")
        self.assertEqual(warning["assets"], ["ALT"])
        self.assertEqual(warning["count"], 2)
        control_codes = [
            warning.get("code") for warning in (self.process_payloads["Control"].get("warnings") or [])
        ]
        self.assertNotIn("legacy_holdings_excluded", control_codes)

    def test_balance_sheet_books_btc_leg_with_execution_basis_and_no_overlay_assets(self):
        rows = self._report("Overlay", "balance-sheet")
        assets = {row["asset"] for row in rows}
        self.assertEqual(assets, {"BTC"})
        btc_row = rows[0]
        self.assertEqual(btc_row["cost_basis"], 20000)

    def test_tax_outputs_match_a_btc_only_control_book(self):
        for report in ("tax-summary", "capital-gains"):
            overlay = self._report("Overlay", report)
            control = self._report("Control", report)
            self.assertEqual(overlay, control, report)

    def test_legacy_holdings_report_shows_overlay_position(self):
        rows = self._report("Overlay", "legacy-holdings")
        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertEqual(row["asset"], "ALT")
        self.assertEqual(row["quantity"], 100.0)
        self.assertIs(row["tax_accounted"], False)
        self.assertEqual(row["priced_at"], "2021-04-01T10:00:00Z")
        self.assertAlmostEqual(row["market_value"], 100 * 20000 / 900, places=6)
        control_rows = self._report("Control", "legacy-holdings")
        self.assertEqual(control_rows, [])

    def test_exit_tax_carries_overlay_notice(self):
        payload, code = _run(
            self.data_root,
            "reports", "exit-tax", "--workspace", "T", "--profile", "Overlay",
            "--departure-date", "2026-01-01", "--destination", "eu_eea",
        )
        self.assertEqual(code, 0, payload)
        assumptions = payload["data"]["assumptions"]
        notice = [line for line in assumptions if line.startswith("EXIT-006")]
        self.assertEqual(len(notice), 1)
        self.assertIn("ALT", notice[0])
        payload, code = _run(
            self.data_root,
            "reports", "exit-tax", "--workspace", "T", "--profile", "Control",
            "--departure-date", "2026-01-01", "--destination", "eu_eea",
        )
        self.assertEqual(code, 0, payload)
        control_assumptions = payload["data"]["assumptions"]
        self.assertFalse([line for line in control_assumptions if line.startswith("EXIT-006")])


if __name__ == "__main__":
    unittest.main()
