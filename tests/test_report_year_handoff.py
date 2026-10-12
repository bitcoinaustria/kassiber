"""`reports export-year-handoff`: one book's year for EA / JA work.

Runs the CLI against a disposable data root. One Austrian book covers an own-
wallet transfer with a fee, a reviewed BTC -> L-BTC peg, an external payment,
income, a transfer still in transit at year end, a row that is 2026 in Vienna
but 2025 in UTC, USDt on Liquid received and spent, an excluded dust deposit,
and an unpriced 2026 deposit that quarantines.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
import zipfile
from decimal import Decimal
from pathlib import Path

from kassiber.core.asset_presentation import describe_asset
from tests.test_cli_smoke import _load_xlsx_sheets


ROOT = Path(__file__).resolve().parent.parent
USDT_LIQUID = "ce091c998b83c78bb71a632313ba3760f1763d9cfcffae02258ffa9865a37bd2"

_HOT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2024-03-01T10:00:00Z,buy-001,inbound,BTC,1.00000000,0,40000,Buy,buy
2025-02-01T10:00:00Z,xfer-001,outbound,BTC,0.50000000,0.00100000,50000,Move to cold,transfer
2025-04-01T10:00:00Z,peg-out-1,outbound,BTC,0.10000000,0.00010000,80000,Peg in,transfer
2025-06-01T10:00:00Z,pay-001,outbound,BTC,0.10000000,0.00010000,60000,Pay supplier,withdrawal
2025-08-01T10:00:00Z,inc-001,inbound,BTC,0.05000000,0,70000,Donation,income
2025-12-31T22:50:00Z,xfer-ye,outbound,BTC,0.20000000,0.00010000,90000,Year-end move,transfer
2025-12-31T23:30:00Z,late-001,outbound,BTC,0.01000000,0,80000,After Vienna midnight,withdrawal
"""

_COLD_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2025-02-01T10:05:00Z,xfer-001,inbound,BTC,0.50000000,0,50000,Move to cold,transfer
2025-12-31T23:10:00Z,xfer-ye,inbound,BTC,0.20000000,0,90000,Year-end move,transfer
"""

_LIQUID_CSV = f"""date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2025-04-01T12:00:00Z,peg-in-1,inbound,LBTC,0.09990000,0,80000,Peg in,transfer
2025-05-01T10:00:00Z,usdt-in-1,inbound,{USDT_LIQUID},500.00000000,0,0.92,Donation in USDt,income
2025-06-01T10:00:00Z,usdt-out-1,outbound,{USDT_LIQUID},100.00000000,0,0.93,Pay supplier in USDt,withdrawal
2025-07-01T10:00:00Z,dust-1,inbound,LBTC,0.00000546,0,60000,Dust,deposit
2026-03-01T10:00:00Z,nopr-001,inbound,USDT,10.00000000,0,,No price,deposit
"""

_SCOPE = ("--workspace", "Main", "--profile", "Verein")
_SHEETS = [
    "Overview",
    "Wallets",
    "Transactions",
    "Journal",
    "Balances",
    "Roll-forward",
    "Reconciliation",
    "Excluded",
]


def _run(data_root, *args):
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--data-root", str(data_root), "--machine", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return json.loads(result.stdout), result.returncode


def _rows(sheet):
    headers = sheet[2]
    return [
        {headers[column]: value for column, value in cells.items() if column in headers}
        for number, cells in sorted(sheet.items())
        if number > 2
    ]



class AccountingYearsTests(unittest.TestCase):
    def _years(self, tax_country, *timestamps):
        import sqlite3

        from kassiber.core.year_handoff import accounting_years

        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute("CREATE TABLE transactions (profile_id TEXT, occurred_at TEXT)")
        conn.executemany("INSERT INTO transactions VALUES ('p', ?)", [(value,) for value in timestamps])
        return accounting_years(conn, {"id": "p", "tax_country": tax_country})

    def test_years_follow_the_book_calendar_not_the_viewer(self):
        # 23:30Z on 31 December 2024 is 2025 in Vienna, still 2024 in UTC.
        self.assertEqual(self._years("at", "2024-12-31T23:30:00Z"), [2025])
        self.assertEqual(self._years("generic", "2024-12-31T23:30:00Z"), [2024])
        self.assertEqual(
            self._years("at", "2021-03-01T10:00:00Z", "2024-12-31T23:30:00Z"),
            [2025, 2024, 2023, 2022, 2021],
        )
        self.assertEqual(self._years("at"), [])

class AssetPresentationTests(unittest.TestCase):
    def test_bitcoin_assets_carry_sats(self):
        btc = describe_asset("BTC")
        self.assertEqual((btc.asset_id, btc.display_code, btc.chain), ("bitcoin:btc", "BTC", "bitcoin"))
        self.assertEqual(btc.quantity(10_010_000_000), Decimal("0.1001"))
        self.assertEqual(btc.exact(10_010_000_000), "0.10010000")
        self.assertEqual(btc.exact(123), "0.00000000123")
        self.assertEqual(btc.exact(-20_000_000_000), "-0.20000000")
        self.assertEqual(btc.sats(10_010_000_500), Decimal("10010000.5"))
        self.assertEqual(describe_asset("l-btc").asset_id, "liquid:lbtc")

    def test_other_assets_group_by_identity_without_sats(self):
        upper = describe_asset(USDT_LIQUID.upper())
        lower = describe_asset(USDT_LIQUID)
        self.assertEqual(upper.asset_id, lower.asset_id)
        self.assertEqual(lower.asset_id, f"liquid:{USDT_LIQUID}")
        self.assertEqual(lower.display_code, "ce091c99…")
        self.assertIsNone(lower.sats(100))
        self.assertEqual(describe_asset("usdt").asset_id, describe_asset("USDT").asset_id)
        extension = describe_asset("ext:tron:usdt")
        self.assertEqual((extension.asset_id, extension.display_code), ("ext:tron:usdt", "USDT"))
        self.assertNotEqual(extension.asset_id, describe_asset("USDT").asset_id)


class YearHandoffExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-year-handoff-")
        tmp = Path(cls._tmp.name)
        cls.root = tmp / "data"

        def run(*args):
            payload, code = _run(cls.root, *args)
            if code != 0:
                raise AssertionError(f"{args} -> {json.dumps(payload)[:400]}")
            return payload["data"]

        run("init")
        run("workspaces", "create", "Main")
        run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", "at", "Verein")
        for label, csv_text in (("Hot", _HOT_CSV), ("Cold", _COLD_CSV), ("Liquid", _LIQUID_CSV)):
            csv_path = tmp / f"{label}.csv"
            csv_path.write_text(csv_text, encoding="utf-8")
            run("wallets", "create", *_SCOPE, "--label", label, "--kind", "custom")
            run("wallets", "import-csv", *_SCOPE, "--wallet", label, "--file", str(csv_path))

        refs = {}
        for label in ("Hot", "Cold", "Liquid"):
            for row in run("transactions", "list", *_SCOPE, "--wallet", label):
                refs[(label, row["external_id"])] = row["id"]
        cls.refs = refs
        for out_key, in_key, kind in (
            (("Hot", "xfer-001"), ("Cold", "xfer-001"), "manual"),
            (("Hot", "xfer-ye"), ("Cold", "xfer-ye"), "manual"),
            (("Hot", "peg-out-1"), ("Liquid", "peg-in-1"), "peg-in"),
        ):
            run(
                "transfers", "pair", *_SCOPE,
                "--tx-out", refs[out_key], "--tx-in", refs[in_key],
                "--kind", kind, "--policy", "carrying-value",
            )
        run(
            "metadata", "exclude", *_SCOPE,
            "--transaction", refs[("Liquid", "dust-1")], "--reason", "Dust attack",
        )
        run("rates", "set", "BTC-EUR", "2025-12-31T22:00:00Z", "90000")
        # 23:00Z is midnight in Vienna: this quote opens 2026, not 2025.
        run("rates", "set", "BTC-EUR", "2025-12-31T23:00:00Z", "95000")
        run("journals", "process", *_SCOPE)

        cls.file = tmp / "handoff-2025.xlsx"
        payload, code = _run(
            cls.root, "reports", "export-year-handoff", *_SCOPE, "--year", "2025", "--file", str(cls.file)
        )
        if code != 0:
            raise AssertionError(json.dumps(payload)[:400])
        cls.payload = payload
        cls.sheets = _load_xlsx_sheets(cls.file)
        cls.transactions = {
            (row["Wallet"], row["Transaction ID"]): row for row in _rows(cls.sheets["Transactions"])
        }

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def test_dashboard_offers_the_book_calendar_years(self):
        import sqlite3

        from kassiber.core.ui_snapshot import build_transactions_dashboard_snapshot

        conn = sqlite3.connect(self.root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        try:
            history = build_transactions_dashboard_snapshot(conn, {"period": "all"})["history"]
        finally:
            conn.close()
        self.assertEqual(history["accountingYears"], [2026, 2025, 2024])

    def test_payload_names_the_period_and_the_open_difference(self):
        self.assertEqual(self.payload["kind"], "reports.export-year-handoff")
        data = self.payload["data"]
        self.assertEqual(data["year"], 2025)
        self.assertEqual(data["calendar"], "Europe/Vienna")
        self.assertEqual(data["period_start"], "2024-12-31T23:00:00Z")
        self.assertEqual(data["period_end"], "2025-12-31T23:00:00Z")
        self.assertEqual(data["sheets"], _SHEETS)
        self.assertEqual(data["rows"]["excluded"], 1)
        self.assertEqual(data["rows"]["wallets"], 3)
        self.assertFalse(data["reconciled"])
        self.assertTrue(any("Reconciliation sheet" in warning for warning in data["warnings"]))
        self.assertEqual(data["report_snapshot"]["report_kind"], "year-handoff.xlsx")
        self.assertEqual(data["report_snapshot"]["period_start_year"], 2025)

    def test_transactions_follow_the_vienna_year_and_carry_classification(self):
        by_key = self.transactions
        self.assertEqual(
            set(by_key),
            {
                ("Hot", "xfer-001"),
                ("Cold", "xfer-001"),
                ("Hot", "peg-out-1"),
                ("Liquid", "peg-in-1"),
                ("Hot", "pay-001"),
                ("Hot", "inc-001"),
                ("Hot", "xfer-ye"),
                ("Liquid", "usdt-in-1"),
                ("Liquid", "usdt-out-1"),
            },
        )
        # 23:30Z on 31 December is already 2026 in Vienna; excluded dust is
        # on its own sheet.
        self.assertNotIn(("Hot", "late-001"), by_key)
        self.assertNotIn(("Liquid", "dust-1"), by_key)
        expected = {
            ("Hot", "xfer-001"): "internal_transfer",
            ("Cold", "xfer-001"): "internal_transfer",
            ("Hot", "peg-out-1"): "swap",
            ("Liquid", "peg-in-1"): "swap",
            ("Hot", "pay-001"): "disposal",
            ("Hot", "inc-001"): "income",
            ("Liquid", "usdt-in-1"): "income",
            ("Liquid", "usdt-out-1"): "disposal",
        }
        for key, classification in expected.items():
            self.assertEqual(by_key[key]["Kassiber Classification"], classification, key)
        self.assertEqual(by_key[("Hot", "pay-001")]["Source Kind"], "withdrawal")
        self.assertEqual(by_key[("Hot", "inc-001")]["Income (EUR)"], 3500.0)

    def test_bitcoin_amounts_carry_units_sats_and_signed_change(self):
        payment = self.transactions[("Hot", "pay-001")]
        self.assertEqual(payment["Asset"], "BTC")
        self.assertEqual(payment["Asset ID"], "bitcoin:btc")
        self.assertEqual(payment["Amount (exact)"], "0.10000000")
        self.assertEqual(payment["Fee (exact)"], "0.00010000")
        self.assertEqual(payment["Net Change (exact)"], "-0.10010000")
        self.assertAlmostEqual(payment["Net Change"], -0.1001, places=11)
        self.assertEqual(payment["Amount (sats)"], 10_000_000)
        self.assertEqual(payment["Net Change (sats)"], -10_010_000)
        self.assertEqual(payment["Proceeds (EUR)"], 6000.0)
        self.assertEqual(payment["Price (EUR per unit)"], 60000.0)

    def test_other_assets_keep_their_own_unit_without_sats(self):
        received = self.transactions[("Liquid", "usdt-in-1")]
        self.assertEqual(received["Asset ID"], f"liquid:{USDT_LIQUID}")
        self.assertEqual(received["Asset"], "ce091c99…")
        self.assertEqual(received["Amount (exact)"], "500.00000000")
        self.assertNotIn("Amount (sats)", received)
        self.assertEqual(received["Income (EUR)"], 460.0)
        self.assertEqual(self.transactions[("Liquid", "usdt-out-1")]["Net Change (exact)"], "-100.00000000")

    def test_both_legs_of_a_pair_share_a_ref_and_name_each_other(self):
        for wallet_out, wallet_in, txid_out, txid_in in (
            ("Hot", "Cold", "xfer-001", "xfer-001"),
            ("Hot", "Liquid", "peg-out-1", "peg-in-1"),
        ):
            out_leg = self.transactions[(wallet_out, txid_out)]
            in_leg = self.transactions[(wallet_in, txid_in)]
            self.assertTrue(out_leg["Pair Ref"])
            self.assertEqual(out_leg["Pair Ref"], in_leg["Pair Ref"])
            self.assertEqual(out_leg["Other Wallet"], wallet_in)
            self.assertEqual(in_leg["Other Wallet"], wallet_out)
            self.assertEqual(out_leg["Other Leg Transaction ID"], txid_in)
            self.assertEqual(in_leg["Other Leg Transaction ID"], txid_out)
        refs = [row.get("Pair Ref") for row in self.transactions.values()]
        for row in self.transactions.values():
            if row["Kassiber Classification"] in {"internal_transfer", "swap"}:
                self.assertTrue(row.get("Pair Ref"), row["Transaction ID"])
            else:
                self.assertFalse(row.get("Pair Ref"), row["Transaction ID"])
        self.assertEqual(len({ref for ref in refs if ref}), 3)

    def test_journal_rows_link_back_to_transactions(self):
        transaction_refs = {row["Transaction Ref"] for row in self.transactions.values()}
        journal = _rows(self.sheets["Journal"])
        self.assertTrue(journal)
        unlinked = {row["Transaction Ref"] for row in journal} - transaction_refs
        # Only the in-transit arrival: booked at send time, observed in 2026.
        self.assertEqual(unlinked, {self.refs[("Cold", "xfer-ye")]})
        for row in journal:
            self.assertNotIn("at_regime", row.get("Description", ""))
        disposal = next(row for row in journal if row["Transaction ID"] == "pay-001")
        self.assertEqual(disposal["Entry Type"], "disposal")
        self.assertEqual(disposal["Quantity (exact)"], "-0.10010000")
        self.assertEqual(disposal["In Capital Gains Report"], 1)

    def test_roll_forward_closes_per_asset(self):
        rows = {row["Asset ID"]: row for row in _rows(self.sheets["Roll-forward"])}
        self.assertEqual(set(rows), {"bitcoin:btc", "liquid:lbtc", f"liquid:{USDT_LIQUID}"})
        for row in rows.values():
            self.assertEqual(row["Quantity Check"], 0)
            self.assertEqual(row["Basis Check (EUR)"], 0)
        btc = rows["bitcoin:btc"]
        self.assertEqual(btc["Opening Quantity (exact)"], "1.00000000")
        self.assertEqual(btc["Opening Cost Basis (EUR)"], 40000.0)
        lbtc = rows["liquid:lbtc"]
        self.assertEqual(lbtc["Closing Quantity (exact)"], "0.09990000")
        # The peg carries basis instead of realizing a gain.
        self.assertEqual(lbtc["Acquired Value (EUR)"], lbtc["Closing Cost Basis (EUR)"])
        self.assertEqual(rows[f"liquid:{USDT_LIQUID}"]["Closing Quantity (exact)"], "400.00000000")

    def test_balances_use_the_year_end_rate(self):
        rows = {(row["Wallet"], row["Asset ID"]): row for row in _rows(self.sheets["Balances"])}
        hot = rows[("Hot", "bitcoin:btc")]
        self.assertEqual(hot["Opening Quantity (exact)"], "1.00000000")
        self.assertEqual(hot["Closing Quantity (exact)"], "0.14870000")
        self.assertEqual(hot["Year-End Rate (EUR per unit)"], 90000.0)
        self.assertEqual(hot["Rate Source"], "manual")
        self.assertEqual(rows[("Cold", "bitcoin:btc")]["Closing Quantity (exact)"], "0.70000000")
        usdt = rows[("Liquid", f"liquid:{USDT_LIQUID}")]
        self.assertEqual(usdt["Rate Source"], "transaction price")
        self.assertEqual(usdt["Year-End Rate (EUR per unit)"], 0.93)

    def test_reconciliation_flags_the_in_transit_transfer(self):
        rows = {(row["Wallet"], row["Asset ID"]): row for row in _rows(self.sheets["Reconciliation"])}
        self.assertEqual(rows[("Hot", "bitcoin:btc")]["Status"], "OK")
        self.assertEqual(rows[("Liquid", "liquid:lbtc")]["Status"], "OK")
        # The journal spells the Liquid asset id differently from the
        # transactions table; one identity still yields one OK row.
        self.assertEqual(rows[("Liquid", f"liquid:{USDT_LIQUID}")]["Status"], "OK")
        cold = rows[("Cold", "bitcoin:btc")]
        self.assertEqual(cold["Status"], "DIFF")
        self.assertEqual(cold["Closing Difference (exact)"], "0.20000000")
        self.assertEqual(cold["Timing Notes"], "xfer-ye: booked 2025-12-31, observed 2026-01-01")
        with zipfile.ZipFile(self.file) as workbook:
            xml = workbook.read(f"xl/worksheets/sheet{_SHEETS.index('Reconciliation') + 1}.xml").decode("utf-8")
        self.assertIn("<f>", xml)
        self.assertIn("IF(AND(", xml)

    def test_wallets_and_excluded_sheets_show_every_wallet_and_exclusion(self):
        wallets = {row["Wallet"]: row for row in _rows(self.sheets["Wallets"])}
        self.assertEqual(set(wallets), {"Hot", "Cold", "Liquid"})
        self.assertEqual(wallets["Hot"]["Transactions In Year"], 5)
        self.assertEqual(wallets["Liquid"]["Excluded In Year"], 1)
        self.assertEqual(wallets["Liquid"]["Assets"], "L-BTC, USDT, ce091c99…")
        excluded = _rows(self.sheets["Excluded"])
        self.assertEqual(len(excluded), 1)
        self.assertEqual(excluded[0]["Transaction ID"], "dust-1")
        self.assertEqual(excluded[0]["Exclusion Reason"], "Dust attack")
        self.assertEqual(excluded[0]["Amount (exact)"], "0.00000546")

    def test_headers_name_the_book_currency(self):
        headers = set(self.sheets["Transactions"][2].values())
        for label in ("Recorded Value (EUR)", "Market Value (EUR)", "Kassiber Classification", "Other Wallet"):
            self.assertIn(label, headers)
        self.assertFalse(any("{fiat}" in str(label) for label in headers))

    def test_preview_fails_on_stale_sources_like_the_export(self):
        import shutil

        from kassiber.cli.handlers import _metadata_hooks, _report_hooks
        from kassiber.core import freshness
        from kassiber.core.export_preview import preview_export
        from kassiber.db import open_db
        from kassiber.errors import AppError

        copy = Path(self._tmp.name) / "stale-copy"
        shutil.copytree(self.root, copy)
        conn = open_db(copy)
        try:
            profile_id = conn.execute("SELECT id FROM profiles").fetchone()["id"]
            freshness.upsert_source_state(
                conn,
                profile_id=profile_id,
                source_key=freshness.source_key(freshness.SOURCE_ONCHAIN, "hot"),
                source_type=freshness.SOURCE_ONCHAIN,
                source_label="Hot",
                status=freshness.STATUS_FRESH,
                blocking_reports=True,
                stale_reason="sync failed",
                checkpoint={},
            )
            conn.commit()
            with self.assertRaises(AppError) as raised:
                preview_export(conn, _report_hooks(), _metadata_hooks(), export="year_handoff", year=2025)
        finally:
            conn.close()
        self.assertEqual(raised.exception.code, "report_freshness_blocked")

    def test_preview_lists_every_sheet_with_its_rows(self):
        from kassiber.cli.handlers import _metadata_hooks, _report_hooks
        from kassiber.core.export_preview import PREVIEW_ROWS, preview_export
        from kassiber.db import open_db

        conn = open_db(self.root)
        try:
            preview = preview_export(conn, _report_hooks(), _metadata_hooks(), export="year_handoff", year=2025)
        finally:
            conn.close()
        self.assertEqual([sheet["name"] for sheet in preview["sheets"]], _SHEETS)
        self.assertFalse(preview["reconciled"])
        self.assertEqual(preview["file_name"], "kassiber-year-handoff-2025.xlsx")
        sheets = {sheet["name"]: sheet for sheet in preview["sheets"]}
        self.assertEqual(sheets["Transactions"]["total"], 9)
        self.assertLessEqual(len(sheets["Transactions"]["rows"]), PREVIEW_ROWS)
        self.assertIn("Market Value (EUR)", sheets["Transactions"]["columns"])
        self.assertTrue(all(isinstance(cell, str) for row in sheets["Balances"]["rows"] for cell in row))

    def test_quarantine_blocks_only_the_years_it_affects(self):
        blocked = Path(self._tmp.name) / "handoff-2026.xlsx"
        payload, code = _run(
            self.root, "reports", "export-year-handoff", *_SCOPE, "--year", "2026", "--file", str(blocked)
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "year_handoff_quarantined")
        self.assertEqual(payload["error"]["details"]["count"], 1)
        self.assertEqual(payload["error"]["details"]["transactions"][0]["transaction_id"], "nopr-001")
        self.assertFalse(blocked.exists())


class YearHandoffFailClosedTests(unittest.TestCase):
    def test_unprocessed_journals_block_the_export(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-year-handoff-stale-") as tmp:
            root = Path(tmp) / "data"
            csv_path = Path(tmp) / "hot.csv"
            csv_path.write_text(_HOT_CSV.splitlines()[0] + "\n" + _HOT_CSV.splitlines()[1] + "\n", encoding="utf-8")
            for args in (
                ("init",),
                ("workspaces", "create", "Main"),
                ("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", "at", "Verein"),
                ("wallets", "create", *_SCOPE, "--label", "Hot", "--kind", "custom"),
                ("wallets", "import-csv", *_SCOPE, "--wallet", "Hot", "--file", str(csv_path)),
            ):
                _, code = _run(root, *args)
                self.assertEqual(code, 0, args)
            target = Path(tmp) / "handoff.xlsx"
            payload, code = _run(
                root, "reports", "export-year-handoff", *_SCOPE, "--year", "2024", "--file", str(target)
            )
            self.assertNotEqual(code, 0)
            self.assertIn("journals process", payload["error"]["message"])
            self.assertFalse(target.exists())


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
