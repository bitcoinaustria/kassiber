"""A price stored in another currency is held, never relabelled.

Changing a book's currency (`profiles set --fiat-currency`, or a country that
requires its own currency) keeps every transaction's stored price and its
currency. Before this fix journal processing read those prices as the new
currency: a EUR purchase at 40,000 reported as 40,000 USD, with nothing
quarantined. Every priced row whose currency is not the book's is now held
as `price_currency_mismatch` until it is repriced in the book's currency, and
the profile update says how many rows that holds. See docs/reference/tax.md
(Quarantines).
"""

from __future__ import annotations

import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, PRICE_CURRENCY_SEMANTICS_MIGRATION
from tests import cli_runner

BOOK_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2024-01-10T12:00:00Z,buy-1,inbound,BTC,1.00000000,0,40000,Buy
2024-06-10T12:00:00Z,sell-1,outbound,BTC,0.50000000,0,60000,Sell
"""

SCOPE = ("--workspace", "Main", "--profile", "Book")


class PriceCurrencyMismatchTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-price-currency-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        # Children must not resolve the owner's real app-data root.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(self.root / "offline-mode.json")
        self.csv = self.root / "book.csv"
        self.csv.write_text(BOOK_CSV, encoding="utf-8")

    def cli(self, data_root: Path, *args: str) -> dict:
        result = cli_runner.run(
            ["--data-root", str(data_root), "--machine", *args],
            capture_output=True,
            text=True,
            env=self.env,
            check=False,
        )
        envelope = json.loads(result.stdout)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotEqual(envelope["kind"], "error", result.stdout)
        return envelope["data"]

    def book(self, name: str, fiat: str, country: str = "generic") -> Path:
        data_root = self.root / name
        self.cli(data_root, "init")
        self.cli(data_root, "workspaces", "create", "Main")
        self.cli(
            data_root, "profiles", "create", "--workspace", "Main",
            "--fiat-currency", fiat, "--tax-country", country, "Book",
        )
        self.cli(data_root, "wallets", "create", *SCOPE, "--label", "W", "--kind", "custom")
        self.cli(data_root, "wallets", "import-csv", *SCOPE, "--wallet", "W", "--file", str(self.csv))
        return data_root

    def assert_switch_holds(self, data_root: Path, update: dict, *, book: str, priced: str) -> None:
        self.assertEqual(update["fiat_currency"], book)
        self.assertEqual(
            update["price_currency_holds"],
            {
                "reason": "price_currency_mismatch",
                "book_currency": book,
                "transactions": 2,
                "by_currency": {priced: 2},
            },
        )
        processed = self.cli(data_root, "journals", "process", *SCOPE)
        self.assertEqual(processed["quarantined"], 2)
        held = self.cli(data_root, "journals", "quarantined", *SCOPE)
        self.assertEqual(
            [item["reason"] for item in held],
            ["price_currency_mismatch", "price_currency_mismatch"],
        )
        for item in held:
            detail = item["detail"]
            self.assertEqual(detail["price_currency"], priced)
            self.assertEqual(detail["book_currency"], book)
            self.assertEqual(detail["required_for"], "book_currency_price")
            self.assertEqual(item["category"], "missing_price")
        # No figure carries a price from another currency.
        self.assertEqual(self.cli(data_root, "reports", "capital-gains", *SCOPE), [])
        self.assertEqual(self.cli(data_root, "reports", "tax-summary", *SCOPE), [])

    def test_a_eur_book_switched_to_usd_holds_its_eur_prices(self):
        data_root = self.book("eur-to-usd", "EUR")
        update = self.cli(data_root, "profiles", "set", *SCOPE, "--fiat-currency", "USD")
        self.assert_switch_holds(data_root, update, book="USD", priced="EUR")

    def test_a_usd_book_switched_to_austria_holds_its_usd_prices(self):
        data_root = self.book("usd-to-at", "USD")
        update = self.cli(data_root, "profiles", "set", *SCOPE, "--tax-country", "at")
        self.assert_switch_holds(data_root, update, book="EUR", priced="USD")

    def test_a_price_override_in_the_book_currency_releases_the_row(self):
        data_root = self.book("override", "EUR")
        self.cli(data_root, "profiles", "set", *SCOPE, "--fiat-currency", "USD")
        self.cli(data_root, "journals", "process", *SCOPE)
        held = {item["transaction_id"]: item for item in self.cli(data_root, "journals", "quarantined", *SCOPE)}
        for transaction_id, rate in zip(sorted(held, key=lambda tid: held[tid]["occurred_at"]), ("43000", "65000")):
            self.cli(
                data_root, "journals", "quarantine", "resolve", "price-override", *SCOPE,
                "--transaction", transaction_id, "--fiat-rate", rate,
            )
        processed = self.cli(data_root, "journals", "process", *SCOPE)
        self.assertEqual(processed["quarantined"], 0)
        gains = self.cli(data_root, "reports", "capital-gains", *SCOPE)
        self.assertEqual(
            [(row["proceeds"], row["cost_basis"], row["gain_loss"]) for row in gains],
            [(32500.0, 21500.0, 11000.0)],
        )

    def test_holdings_are_never_valued_at_a_price_in_another_currency(self):
        # The buy is repriced in USD; the later EUR sale stays held. With no
        # market quote cached, holdings fall back to the latest transaction
        # price, which must be the USD one, not the sale's 60,000 EUR.
        data_root = self.book("valuation", "EUR")
        self.cli(data_root, "profiles", "set", *SCOPE, "--fiat-currency", "USD")
        self.cli(data_root, "journals", "process", *SCOPE)
        held = self.cli(data_root, "journals", "quarantined", *SCOPE)
        buy = next(item["transaction_id"] for item in held if item["occurred_at"].startswith("2024-01"))
        self.cli(
            data_root, "journals", "quarantine", "resolve", "price-override", *SCOPE,
            "--transaction", buy, "--fiat-rate", "43000",
        )
        self.assertEqual(self.cli(data_root, "journals", "process", *SCOPE)["quarantined"], 1)
        for report in ("balance-sheet", "portfolio-summary"):
            rows = self.cli(data_root, "reports", report, *SCOPE)
            self.assertEqual([(row["quantity"], row["market_value"]) for row in rows], [(1.0, 43000.0)], report)

    def test_opening_a_processed_mismatched_book_requires_one_rebuild(self):
        # A book switched and processed before the fix: its stored journals
        # read EUR prices as USD. Opening it once invalidates exactly those
        # journals; an unaffected processed book keeps its own.
        affected = self.book("legacy-affected", "EUR")
        untouched = self.book("legacy-untouched", "EUR")
        for data_root in (affected, untouched):
            self.cli(data_root, "journals", "process", *SCOPE)
        for data_root, currency in ((affected, "USD"), (untouched, "EUR")):
            with sqlite3.connect(data_root / "kassiber.sqlite3") as conn:
                conn.execute(
                    "UPDATE profiles SET fiat_currency = ?, journal_semantics_version = 2,"
                    " required_journal_semantics_version = 2",
                    (currency,),
                )
                conn.execute(
                    "DELETE FROM schema_migration_audits WHERE migration_name = ?",
                    (PRICE_CURRENCY_SEMANTICS_MIGRATION,),
                )
        self.cli(affected, "profiles", "get", *SCOPE)
        self.cli(untouched, "profiles", "get", *SCOPE)
        with sqlite3.connect(affected / "kassiber.sqlite3") as conn:
            processed_at, required = conn.execute(
                "SELECT last_processed_at, required_journal_semantics_version FROM profiles"
            ).fetchone()
            impact = json.loads(conn.execute(
                "SELECT impact_json FROM schema_migration_audits WHERE migration_name = ?",
                (PRICE_CURRENCY_SEMANTICS_MIGRATION,),
            ).fetchone()[0])
        self.assertIsNone(processed_at)
        self.assertEqual(required, JOURNAL_CALCULATION_SEMANTICS_VERSION)
        self.assertEqual(impact["affected_profile_count"], 1)
        self.assertEqual(impact["required_journal_semantics_version"], JOURNAL_CALCULATION_SEMANTICS_VERSION)
        with sqlite3.connect(untouched / "kassiber.sqlite3") as conn:
            processed_at, required = conn.execute(
                "SELECT last_processed_at, required_journal_semantics_version FROM profiles"
            ).fetchone()
        self.assertIsNotNone(processed_at)
        self.assertEqual(required, 2)
        processed = self.cli(affected, "journals", "process", *SCOPE)
        self.assertEqual(processed["quarantined"], 2)
        self.assertEqual(self.cli(affected, "reports", "capital-gains", *SCOPE), [])

    def test_an_unchanged_currency_reports_nothing_held(self):
        data_root = self.book("same", "EUR")
        update = self.cli(data_root, "profiles", "set", *SCOPE, "--tax-country", "at")
        self.assertNotIn("price_currency_holds", update)
        self.assertEqual(self.cli(data_root, "journals", "process", *SCOPE)["quarantined"], 0)


if __name__ == "__main__":
    unittest.main()
