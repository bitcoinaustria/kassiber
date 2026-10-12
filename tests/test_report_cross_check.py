"""`reports cross-check`: a CoinTracking or Blockpit export against the book.

The first book is the platform-export fixture (buy, sale, own-wallet transfer,
peg, payment, income, mining, USDt on Liquid). Kassiber's own export of it must
check back as fully matched; edited copies must surface each kind of
difference. The second book is never processed and has an excluded row, a
Vienna year-boundary row and a tool export kept two hours ahead of UTC.
"""

from __future__ import annotations

import csv
import json
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from tests.test_report_transaction_formats import (
    _COLD_CSV,
    _HOT_CSV,
    _LIQUID_CSV,
    _SCOPE,
    _read_csv,
    _run,
)

_WALLETS = ("--wallet", "Hot", "--wallet", "Cold", "--wallet", "Liquid")


def _write_csv(path, headers, rows):
    with Path(path).open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=headers)
        writer.writeheader()
        writer.writerows(rows)


def _by_txid(result):
    out = {}
    for row in result["rows"]:
        side = row["kassiber"] or row["tool"]
        out[side["txid"] or f"row-{row['tool']['row']}"] = row
    return out


class CrossCheckRoundTripTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-cross-check-")
        tmp = Path(cls._tmp.name)
        cls.tmp = tmp
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
        for out_key, in_key, kind in (
            (("Hot", "xfer-001"), ("Cold", "xfer-001"), "manual"),
            (("Hot", "peg-out-1"), ("Liquid", "peg-in-1"), "peg-in"),
        ):
            run(
                "transfers", "pair", *_SCOPE,
                "--tx-out", refs[out_key], "--tx-in", refs[in_key],
                "--kind", kind, "--policy", "carrying-value",
            )
        run("journals", "process", *_SCOPE)
        cls.files = {}
        for platform in ("cointracking", "blockpit"):
            target = tmp / f"{platform}.csv"
            run("transactions", "export", *_SCOPE, *_WALLETS, "--export-format", f"{platform}-csv", "--file", str(target))
            cls.files[platform] = target

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def check(self, tool, path, *extra):
        payload, code = _run(self.root, "reports", "cross-check", *_SCOPE, "--tool", tool, "--file", str(path), *extra)
        self.assertEqual(code, 0, json.dumps(payload)[:600])
        self.assertEqual(payload["kind"], "reports.cross-check")
        return payload["data"]

    def test_kassibers_own_exports_check_back_as_matched(self):
        for tool in ("cointracking", "blockpit"):
            with self.subTest(tool=tool):
                result = self.check(tool, self.files[tool])
                self.assertTrue(result["agrees"], [row for row in result["rows"] if row["status"] != "matched"])
                self.assertEqual(result["counts"]["matched"], 9)
                self.assertEqual(result["tax_differences"], 0)
                self.assertTrue(all(row["match"] == "txid" for row in result["rows"]))
                self.assertTrue(all(total["agrees"] is not False for total in result["totals"]), result["totals"])
                income = next(total for total in result["totals"] if total["metric"] == "income_value")
                # CoinTracking carries values; Blockpit's template does not.
                self.assertEqual(income["agrees"], True if tool == "cointracking" else None)
                codes = {insight["code"] for insight in result["insights"]}
                # USDt is exported by asset id, which neither tool parser reads.
                self.assertIn("tool_other_assets", codes)
                self.assertIn("kassiber_other_assets", codes)
                legs = [row for row in result["rows"] if row["kassiber"]["txid"] == "xfer-001"]
                self.assertEqual(
                    sorted((row["kassiber"]["wallet"], row["tool"]["type"]) for row in legs),
                    [("Cold", "Deposit"), ("Hot", "Withdrawal")],
                )
                self.assertTrue(all(row["kassiber"]["classification"] == "internal_transfer" for row in legs))

    def test_edits_in_the_tool_show_as_differences(self):
        headers, rows = _read_csv(self.files["cointracking"])
        edited = []
        for row in rows:
            txid = row["Tx-ID"]
            if txid == "mine-001":
                continue  # dropped in the tool
            if txid == "inc-001":
                row["Buy Amount"] = "0.04000000"
            if txid == "pay-001":
                row["Type"] = "Withdrawal"  # a payment the tool calls a transfer
            if txid == "sell-001":
                row["Tx-ID"] = ""  # no id: matched by amount and time
                moved = datetime.strptime(row["Date"], "%Y-%m-%d %H:%M:%S") + timedelta(hours=1)
                row["Date"] = moved.strftime("%Y-%m-%d %H:%M:%S")
            edited.append(row)
        edited.append(
            {
                **{name: "" for name in headers},
                "Type": "Deposit",
                "Buy Amount": "0.20000000",
                "Buy Currency": "BTC",
                "Exchange": "Kraken",
                "Date": "2025-10-01 09:00:00",
                "Tx-ID": "extra-1",
            }
        )
        path = self.tmp / "edited.csv"
        _write_csv(path, headers, edited)
        result = self.check("cointracking", path)
        rows_by_txid = _by_txid(result)
        self.assertFalse(result["agrees"])

        income = rows_by_txid["inc-001"]
        self.assertEqual(income["status"], "different")
        self.assertEqual([diff["field"] for diff in income["differences"]], ["amount"])
        self.assertEqual(income["differences"][0]["delta"], "-0.01000000")

        payment = rows_by_txid["pay-001"]
        self.assertEqual(payment["status"], "different")
        self.assertEqual(payment["differences"][0]["field"], "type")
        self.assertEqual(payment["differences"][0]["kassiber"], "disposal")

        self.assertEqual(rows_by_txid["mine-001"]["status"], "only_kassiber")
        self.assertEqual(rows_by_txid["extra-1"]["status"], "only_tool")
        self.assertEqual(rows_by_txid["extra-1"]["tool"]["source"], "Kraken")

        sale = rows_by_txid["sell-001"]
        self.assertEqual((sale["status"], sale["match"]), ("matched", "probable"))
        self.assertEqual([(d["field"], d["severity"], d["delta"]) for d in sale["differences"]], [("time", "info", "60")])

        self.assertEqual(result["counts"]["different"], 2)
        self.assertEqual(result["tax_differences"], 2)
        inbound = next(t for t in result["totals"] if t["asset"] == "BTC" and t["metric"] == "inbound")
        # +0.2 extra deposit, -0.01 income, -0.01 dropped mining
        self.assertEqual(inbound["delta"], "0.18000000")
        self.assertFalse(inbound["agrees"])
        # The tool's wallet names pair with Kassiber wallets; a name Kassiber
        # does not know is one finding, not row problems in a known wallet.
        pairs = {pair["key"]: pair for pair in result["pairs"]}
        hot = pairs["wallet:Hot"]
        self.assertEqual((hot["state"], hot["tool_names"], hot["issues"]), ("paired", ["Hot"], 3))
        kraken = pairs["tool:Kraken"]
        self.assertEqual((kraken["state"], kraken["wallet"], kraken["rows"], kraken["issues"]), ("tool_only", None, 1, 0))
        self.assertEqual(result["pairs"][-1]["key"], "tool:Kraken")
        self.assertTrue(all(row["pair"] in pairs for row in result["rows"]))
        self.assertEqual(rows_by_txid["extra-1"]["pair"], "tool:Kraken")
        self.assertEqual(rows_by_txid["mine-001"]["pair"], "wallet:Hot")

        only_hot, _ = _run(
            self.root, "reports", "cross-check", *_SCOPE, "--tool", "cointracking",
            "--file", str(path), "--source", "Kraken",
        )
        filtered = only_hot["data"]
        self.assertEqual(filtered["counts"]["only_tool"], 1)
        self.assertEqual(filtered["counts"]["matched"], 0)
        self.assertEqual(
            {pair["key"]: pair["state"] for pair in filtered["pairs"]}["wallet:Hot"], "kassiber_only"
        )

        workbook_path = self.tmp / "edited.xlsx"
        payload, code = _run(
            self.root, "reports", "cross-check", *_SCOPE, "--tool", "cointracking",
            "--file", str(path), "--xlsx", str(workbook_path),
        )
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload["data"]["tax_differences"], 2)
        from tests.test_cli_smoke import _load_xlsx_sheets

        sheets = _load_xlsx_sheets(workbook_path)
        self.assertEqual(
            list(sheets)[:6],
            ["Summary", "Differences", "Only in tool", "Only in Kassiber", "Excluded in Kassiber", "Matched"],
        )
        self.assertEqual(len(sheets["Differences"]), 3)  # header + 2 rows

    def test_an_unreadable_row_is_named_not_dropped(self):
        headers, rows = _read_csv(self.files["blockpit"])
        rows[0]["Label"] = "Margin Trade Settlement"
        path = self.tmp / "unreadable.csv"
        _write_csv(path, headers, rows)
        result = self.check("blockpit", path)
        self.assertEqual(result["counts"]["not_comparable"], 1)
        broken = next(row for row in result["rows"] if row["status"] == "not_comparable")
        self.assertIn("needs review", broken["reason"])
        self.assertEqual(result["counts"]["only_kassiber"], 1)

    def test_wrong_file_and_tool_fail_with_a_hint(self):
        payload, code = _run(
            self.root, "reports", "cross-check", *_SCOPE, "--tool", "blockpit",
            "--file", str(self.files["cointracking"]),
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("Blockpit CSV is missing required columns", payload["error"]["message"])


_SIDE_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2024-12-31T23:30:00Z,eve-1,inbound,BTC,0.01000000,0,90000,Silvester,deposit
2025-03-01T10:00:00Z,in-1,inbound,BTC,0.30000000,0,80000,Deposit,deposit
2025-05-01T10:00:00Z,in-2,inbound,BTC,0.20000000,0,85000,Deposit,deposit
2025-06-01T10:00:00Z,out-1,outbound,BTC,0.10000000,0.00010000,90000,Withdrawal,withdrawal
2025-07-01T10:00:00Z,dust-1,inbound,BTC,0.00000546,0,90000,Dust,deposit
2025-07-01T10:00:01Z,dust-2,inbound,BTC,0.00000546,0,90000,Dust twin,deposit
"""


class CrossCheckUnprocessedBookTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-cross-check-side-")
        tmp = Path(cls._tmp.name)
        cls.tmp = tmp
        cls.root = tmp / "data"

        def run(*args):
            payload, code = _run(cls.root, *args)
            if code != 0:
                raise AssertionError(f"{args} -> {json.dumps(payload)[:400]}")
            return payload["data"]

        run("init")
        run("workspaces", "create", "Main")
        run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", "at", "Verein")
        source = tmp / "side.csv"
        source.write_text(_SIDE_CSV, encoding="utf-8")
        run("wallets", "create", *_SCOPE, "--label", "Ledger", "--kind", "custom")
        run("wallets", "import-csv", *_SCOPE, "--wallet", "Ledger", "--file", str(source))
        dust = next(row for row in run("transactions", "list", *_SCOPE) if row["external_id"] == "dust-1")
        run("metadata", "exclude", *_SCOPE, "--transaction", dust["id"], "--reason", "Dust attack")
        # The tool account keeps local time two hours ahead of UTC.
        cls.file = tmp / "blockpit.csv"
        _write_csv(
            cls.file,
            ["Date (UTC)", "Integration Name", "Label", "Outgoing Asset", "Outgoing Amount",
             "Incoming Asset", "Incoming Amount", "Fee Asset", "Fee Amount", "Trx. ID"],
            [
                {"Date (UTC)": "01.03.2025 12:00:00", "Integration Name": "Ledger", "Label": "Deposit",
                 "Incoming Asset": "BTC", "Incoming Amount": "0.3", "Trx. ID": "in-1"},
                {"Date (UTC)": "01.05.2025 12:00:00", "Integration Name": "Ledger", "Label": "Deposit",
                 "Incoming Asset": "BTC", "Incoming Amount": "0.2", "Trx. ID": "IN-2"},
                {"Date (UTC)": "01.06.2025 12:00:00", "Integration Name": "Ledger", "Label": "Withdrawal",
                 "Outgoing Asset": "BTC", "Outgoing Amount": "0.1", "Fee Asset": "BTC", "Fee Amount": "0.0001",
                 "Trx. ID": "out-1"},
                # The excluded row's own id: it must not pair with its
                # active twin of the same amount one second later.
                {"Date (UTC)": "01.07.2025 12:00:00", "Integration Name": "Ledger", "Label": "Deposit",
                 "Incoming Asset": "BTC", "Incoming Amount": "0.00000546", "Trx. ID": "dust-1"},
            ],
        )

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def check(self, *extra):
        payload, code = _run(
            self.root, "reports", "cross-check", *_SCOPE, "--tool", "blockpit", "--file", str(self.file), *extra
        )
        self.assertEqual(code, 0, json.dumps(payload)[:600])
        return payload["data"]

    def test_clock_offset_is_reported_once_and_excluded_rows_are_named(self):
        result = self.check("--year", "2025")
        insights = {insight["code"]: insight for insight in result["insights"]}
        self.assertEqual(insights["clock_offset"]["params"], {"hours": 2})
        self.assertIn("journals_stale", insights)
        self.assertEqual(result["counts"]["matched"], 3)
        self.assertFalse(any(row["differences"] for row in result["rows"] if row["status"] == "matched"))
        excluded = next(row for row in result["rows"] if row["status"] == "excluded_in_kassiber")
        self.assertEqual(excluded["kassiber"]["txid"], "dust-1")
        self.assertTrue(excluded["kassiber"]["excluded"])
        # 23:30Z on 31 December 2024 is already 2025 in Vienna.
        only = [row["kassiber"]["txid"] for row in result["rows"] if row["status"] == "only_kassiber"]
        self.assertEqual(only, ["eve-1", "dust-2"])
        self.assertEqual(result["calendar"], "Europe/Vienna")

    def test_other_years_leave_the_year_out(self):
        result = self.check("--year", "2024")
        self.assertEqual(result["rows"], [])
        self.assertEqual(self.check("--year", "2026")["rows"], [])

    def test_wallets_imported_from_the_same_tool_are_skipped(self):
        payload, code = _run(self.root, "wallets", "create", *_SCOPE, "--label", "Blockpit import", "--kind", "blockpit")
        self.assertEqual(code, 0, payload)
        try:
            result = self.check()
            skipped = next(i for i in result["insights"] if i["code"] == "skipped_tool_wallets")
            self.assertEqual(skipped["params"]["wallets"], ["Blockpit import"])
        finally:
            _run(self.root, "wallets", "delete", *_SCOPE, "--wallet", "Blockpit import")


class MatchingRuleTests(unittest.TestCase):
    """The rules that decide whether two rows are the same transaction."""

    @staticmethod
    def _tool(**over):
        from kassiber.core.cross_check import _ToolRow

        values = dict(index=1, source="Ledger", label="Deposit", txid="", occurred_at=None,
                      direction="inbound", asset_id="bitcoin:btc", amount_msat=10_000_000_000, kind="deposit")
        values.update(over)
        return _ToolRow(**values)

    @staticmethod
    def _book(**over):
        from kassiber.core.cross_check import _BookRow

        values = dict(id="tx-1", wallet="Ledger", wallet_id="w", txid="", occurred_at=None, direction="inbound",
                      asset_id="bitcoin:btc", principal_msat=10_000_000_000, fee_msat=0,
                      classification="acquisition", value=None, excluded=False)
        values.update(over)
        return _BookRow(**values)

    def test_ordinary_drift_is_not_a_timezone(self):
        from kassiber.core.cross_check import _detect_offset

        base = datetime(2025, 3, 1, 10, 0, tzinfo=timezone.utc)
        drift = [(self._tool(occurred_at=base + timedelta(minutes=31)), self._book(occurred_at=base))] * 3
        self.assertEqual(_detect_offset(drift), timedelta(0))
        shifted = [(self._tool(occurred_at=base + timedelta(hours=2, minutes=1)), self._book(occurred_at=base))] * 3
        self.assertEqual(_detect_offset(shifted), timedelta(hours=2))

    def test_two_different_chain_ids_are_never_paired_by_amount(self):
        from kassiber.core.cross_check import _Index

        moment = datetime(2025, 3, 1, 10, 0, tzinfo=timezone.utc)
        book = self._book(occurred_at=moment, txid="a" * 64)
        index = _Index([book])
        self.assertIsNone(index.probable(self._tool(occurred_at=moment, txid="b" * 64), timedelta(0)))
        self.assertIs(index.probable(self._tool(occurred_at=moment, txid="KRAKEN-1"), timedelta(0)), book)

    def test_a_different_tax_year_counts_however_small_the_drift(self):
        from zoneinfo import ZoneInfo

        from kassiber.core.cross_check import _differences

        book = self._book(occurred_at=datetime(2024, 12, 31, 22, 59, tzinfo=timezone.utc))
        tool = self._tool(occurred_at=datetime(2024, 12, 31, 23, 1, tzinfo=timezone.utc))
        vienna = _differences(tool, book, offset=timedelta(0), calendar=ZoneInfo("Europe/Vienna"), compare_types=False)
        self.assertEqual([(diff["field"], diff["severity"]) for diff in vienna], [("time", "tax")])
        utc = _differences(tool, book, offset=timedelta(0), calendar=ZoneInfo("UTC"), compare_types=False)
        self.assertEqual(utc, [])


if __name__ == "__main__":
    unittest.main()
