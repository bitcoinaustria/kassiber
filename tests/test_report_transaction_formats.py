"""`transactions export --export-format cointracking-csv|blockpit-csv`.

One book with a buy, a sale, an own-wallet transfer with a fee, a reviewed
BTC -> L-BTC peg, a payment, income and mining, USDt on Liquid, and a separate
wallet holding a quarantined row. The CSVs are checked against each tool's
import vocabulary and read back through Kassiber's own importers.
"""

from __future__ import annotations

import csv
import json
import subprocess
import sys
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from kassiber.importers import normalize_blockpit_record, normalize_cointracking_record

ROOT = Path(__file__).resolve().parent.parent
USDT_LIQUID = "ce091c998b83c78bb71a632313ba3760f1763d9cfcffae02258ffa9865a37bd2"
_SCOPE = ("--workspace", "Main", "--profile", "Verein")

_HOT_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,description,kind
2025-01-10T10:00:00Z,buy-001,inbound,BTC,1.00000000,0,40000,40000,Buy on exchange,buy
2025-02-01T10:00:00Z,xfer-001,outbound,BTC,0.50000000,0.00100000,,50000,Move to cold,transfer
2025-04-01T10:00:00Z,peg-out-1,outbound,BTC,0.10000000,0.00010000,,80000,Peg in,transfer
2025-06-01T10:00:00Z,pay-001,outbound,BTC,0.10000000,0.00010000,,60000,Pay supplier,withdrawal
2025-07-01T10:00:00Z,sell-001,outbound,BTC,0.05000000,0,3200,64000,Sell for EUR,sell
2025-08-01T10:00:00Z,inc-001,inbound,BTC,0.05000000,0,,70000,Donation,income
2025-09-01T10:00:00Z,mine-001,inbound,BTC,0.01000000,0,,72000,Pool payout,mining
"""

_COLD_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2025-02-01T10:05:00Z,xfer-001,inbound,BTC,0.50000000,0,50000,Move to cold,transfer
"""

_LIQUID_CSV = f"""date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2025-04-01T12:00:00Z,peg-in-1,inbound,LBTC,0.09990000,0,80000,Peg in,transfer
2025-05-01T10:00:00Z,usdt-in-1,inbound,{USDT_LIQUID},500.00000000,0,0.92,Donation in USDt,income
"""

_EXCHANGE_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description,kind
2025-03-01T10:00:00Z,nopr-001,inbound,USDT,10.00000000,0,,No price,deposit
"""


def _run(data_root, *args):
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--data-root", str(data_root), "--machine", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    return json.loads(result.stdout), result.returncode


def _read_csv(path):
    with Path(path).open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        return reader.fieldnames, list(reader)


class PlatformExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-platform-export-")
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
        for label, csv_text in (
            ("Hot", _HOT_CSV),
            ("Cold", _COLD_CSV),
            ("Liquid", _LIQUID_CSV),
            ("Exchange", _EXCHANGE_CSV),
        ):
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

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _export(self, platform, wallet=None):
        wallets = [wallet] if isinstance(wallet, str) else list(wallet or [])
        target = self.tmp / f"{platform}-{'-'.join(wallets) or 'all'}.csv"
        args = ["transactions", "export", *_SCOPE, "--export-format", f"{platform}-csv", "--file", str(target)]
        for name in wallets:
            args += ["--wallet", name]
        payload, code = _run(self.root, *args)
        return payload, code, target

    def _rows_by_txid(self, platform, wallet, txid_column):
        payload, code, target = self._export(platform, wallet)
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        self.assertEqual(payload["kind"], "transactions.export")
        headers, rows = _read_csv(target)
        return payload["data"], headers, {(row[txid_column]): row for row in rows}

    def test_cointracking_rows_use_its_import_vocabulary(self):
        data, headers, rows = self._rows_by_txid("cointracking", "Hot", "Tx-ID")
        self.assertEqual(headers[:11], [
            "Type", "Buy Amount", "Buy Currency", "Sell Amount", "Sell Currency",
            "Fee", "Fee Currency", "Exchange", "Trade-Group", "Comment", "Date",
        ])
        self.assertEqual(data["platform"], "cointracking")
        self.assertEqual(data["rows"], 7)
        buy = rows["buy-001"]
        self.assertEqual((buy["Type"], buy["Buy Amount"], buy["Buy Currency"]), ("Trade", "1.00000000", "BTC"))
        self.assertEqual((buy["Sell Amount"], buy["Sell Currency"]), ("40000", "EUR"))
        sale = rows["sell-001"]
        self.assertEqual((sale["Type"], sale["Sell Amount"], sale["Buy Amount"], sale["Buy Currency"]),
                         ("Trade", "0.05000000", "3200", "EUR"))
        transfer = rows["xfer-001"]
        self.assertEqual((transfer["Type"], transfer["Sell Amount"], transfer["Fee"], transfer["Fee Currency"]),
                         ("Withdrawal", "0.50000000", "0.00100000", "BTC"))
        self.assertIn("internal_transfer", transfer["Comment"])
        self.assertIn("pair ", transfer["Comment"])
        self.assertEqual(rows["pay-001"]["Type"], "Spend")
        self.assertEqual(rows["inc-001"]["Type"], "Income")
        self.assertEqual(rows["inc-001"]["Buy Value in Account Currency"], "3500")
        self.assertEqual(rows["mine-001"]["Type"], "Mining")
        self.assertEqual(rows["peg-out-1"]["Type"], "Withdrawal")
        self.assertEqual(buy["Exchange"], "Hot")
        self.assertEqual(buy["Date"], "2025-01-10 10:00:00")
        self.assertTrue(any("swap legs" in warning for warning in data["warnings"]))

    def test_blockpit_rows_follow_the_manual_import_template(self):
        data, headers, rows = self._rows_by_txid("blockpit", "Hot", "Trx. ID (optional)")
        self.assertEqual(headers, [
            "Date (UTC)", "Integration Name", "Label", "Outgoing Asset", "Outgoing Amount",
            "Incoming Asset", "Incoming Amount", "Fee Asset (optional)", "Fee Amount (optional)",
            "Comment (optional)", "Trx. ID (optional)",
        ])
        buy = rows["buy-001"]
        self.assertEqual((buy["Label"], buy["Outgoing Asset"], buy["Outgoing Amount"]), ("Trade", "EUR", "40000"))
        self.assertEqual((buy["Incoming Asset"], buy["Incoming Amount"]), ("BTC", "1.00000000"))
        self.assertEqual(buy["Date (UTC)"], "10.01.2025 10:00:00")
        self.assertEqual(rows["pay-001"]["Label"], "Payment")
        self.assertEqual(rows["xfer-001"]["Label"], "Withdrawal")
        self.assertEqual(rows["xfer-001"]["Fee Amount (optional)"], "0.00100000")
        self.assertEqual(rows["inc-001"]["Label"], "Income")
        self.assertEqual(rows["mine-001"]["Label"], "Mining")
        self.assertEqual(data["rows"], 7)

    def test_exports_read_back_through_kassibers_importers(self):
        for platform, txid_column, normalize in (
            ("cointracking", "Tx-ID", normalize_cointracking_record),
            ("blockpit", "Trx. ID (optional)", normalize_blockpit_record),
        ):
            _, _, rows = self._rows_by_txid(platform, "Hot", txid_column)
            expected = {
                "buy-001": ("inbound", Decimal("1"), Decimal("0"), "buy"),
                "sell-001": ("outbound", Decimal("0.05"), Decimal("0"), "sell"),
                "xfer-001": ("outbound", Decimal("0.5"), Decimal("0.001"), "withdrawal"),
                "pay-001": ("outbound", Decimal("0.1"), Decimal("0.0001"), "spend"),
                "inc-001": ("inbound", Decimal("0.05"), Decimal("0"), "income"),
                "mine-001": ("inbound", Decimal("0.01"), Decimal("0"), "mining"),
            }
            for index, (txid, (direction, amount, fee, kind)) in enumerate(expected.items(), start=1):
                record = normalize(rows[txid], index=index)
                self.assertEqual(record["direction"], direction, (platform, txid))
                self.assertEqual(record["amount"], amount, (platform, txid))
                self.assertEqual(record["fee"], fee, (platform, txid))
                self.assertEqual(record["kind"], kind, (platform, txid))
                self.assertEqual(record["txid"], txid, (platform, txid))
            trade = normalize(rows["buy-001"], index=1)
            self.assertEqual((trade["fiat_value"], trade["fiat_currency"]), (Decimal("40000"), "EUR"))

    def test_other_assets_export_with_their_id_and_a_warning(self):
        data, _, rows = self._rows_by_txid("cointracking", "Liquid", "Tx-ID")
        usdt = rows["usdt-in-1"]
        self.assertEqual((usdt["Type"], usdt["Buy Amount"], usdt["Buy Currency"]),
                         ("Income", "500.00000000", USDT_LIQUID))
        self.assertEqual(rows["peg-in-1"]["Buy Currency"], "LBTC")
        self.assertTrue(any(USDT_LIQUID in warning for warning in data["warnings"]))

    def test_a_quarantine_blocks_only_the_scope_it_is_in(self):
        payload, code, target = self._export("blockpit")
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "transaction_export_quarantined")
        self.assertEqual(payload["error"]["details"]["transactions"][0]["transaction_id"], "nopr-001")
        self.assertFalse(target.exists())
        payload, code, _ = self._export("blockpit", "Exchange")
        self.assertEqual(payload["error"]["code"], "transaction_export_quarantined")
        payload, code, _ = self._export("blockpit", "Cold")
        self.assertEqual(code, 0, json.dumps(payload)[:300])

    def _preview(self, **kwargs):
        from kassiber.cli.handlers import _metadata_hooks, _report_hooks
        from kassiber.core.export_preview import preview_export
        from kassiber.db import open_db

        conn = open_db(self.root)
        try:
            return preview_export(conn, _report_hooks(), _metadata_hooks(), **kwargs)
        finally:
            conn.close()

    def test_preview_shows_the_file_the_export_writes(self):
        preview = self._preview(export="transactions", export_format="blockpit", wallet_ref="Hot")
        _, headers, rows = self._rows_by_txid("blockpit", "Hot", "Trx. ID (optional)")
        sheet = preview["sheets"][0]
        self.assertEqual(sheet["columns"], headers)
        self.assertEqual(preview["row_count"], 7)
        self.assertEqual(sheet["total"], 7)
        first = dict(zip(headers, sheet["rows"][0]))
        self.assertEqual(first, rows[first["Trx. ID (optional)"]])
        self.assertEqual(preview["file_name"], "kassiber-blockpit.csv")
        workbook = self._preview(export="transactions", export_format="xlsx", wallet_ref="Hot")
        self.assertIn("Transaction ID", workbook["sheets"][0]["columns"])
        self.assertEqual(workbook["row_count"], 7)

    def test_preview_fails_closed_like_the_export(self):
        from kassiber.errors import AppError

        with self.assertRaises(AppError) as raised:
            self._preview(export="transactions", export_format="cointracking")
        self.assertEqual(raised.exception.code, "transaction_export_quarantined")

    def test_label_preview_shortens_key_material(self):
        from unittest import mock

        from kassiber.core import export_preview

        xpub = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz"
        records = [
            {"type": "xpub", "ref": xpub, "label": "Cold storage", "origin": "wpkh([d34db33f/84'/0'/0'])"},
            {"type": "tx", "ref": "ab" * 32, "label": "Rent"},
        ]
        with mock.patch.object(
            export_preview.core_metadata,
            "bip329_export_records",
            return_value={"records": records, "mode": "all", "wallet": None,
                          "exported_stored": 2, "exported_synthesized": 0},
        ):
            preview = self._preview(export="labels")
        rows = preview["sheets"][0]["rows"]
        self.assertEqual(preview["sheets"][0]["columns"], ["type", "ref", "label"])
        self.assertNotIn(xpub, json.dumps(preview))
        self.assertNotIn("d34db33f", json.dumps(preview))
        self.assertEqual(rows[0][1], f"{xpub[:6]}…{xpub[-4:]}")
        self.assertEqual(rows[1], ["tx", "ab" * 32, "Rent"])

    def test_several_wallets_export_together_without_the_rest(self):
        payload, code, target = self._export("blockpit", ["Hot", "Cold"])
        self.assertEqual(code, 0, json.dumps(payload)[:300])
        self.assertEqual(payload["data"]["rows"], 8)
        self.assertEqual(payload["data"]["wallet"], "Hot, Cold")
        _, rows = _read_csv(target)
        self.assertEqual({row["Integration Name"] for row in rows}, {"Hot", "Cold"})
        preview = self._preview(export="transactions", export_format="xlsx", wallet_refs=["Hot", "Cold"])
        self.assertEqual(preview["row_count"], 8)

    def test_an_empty_wallet_selection_is_rejected(self):
        from kassiber.errors import AppError

        with self.assertRaises(AppError) as raised:
            self._preview(export="transactions", export_format="csv", wallet_refs=[])
        self.assertEqual(raised.exception.code, "validation")

    def test_labels_merge_across_selected_wallets(self):
        from kassiber.cli.handlers import _metadata_hooks
        from kassiber.core.metadata import bip329_export_records
        from kassiber.db import open_db

        conn = open_db(self.root)
        try:
            both = bip329_export_records(conn, None, None, _metadata_hooks(), wallet_refs=["Hot", "Cold"], mode="all")
            hot = bip329_export_records(conn, None, None, _metadata_hooks(), wallet_refs=["Hot"], mode="all")
        finally:
            conn.close()
        self.assertEqual([wallet["label"] for wallet in both["wallets"]], ["Hot", "Cold"])
        keys = [(record["type"], record["ref"]) for record in both["records"]]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertGreaterEqual(len(keys), len(hot["records"]))

    def test_a_period_narrows_the_file_and_its_quarantine_barrier(self):
        # The Exchange wallet's quarantine is dated March; June to July
        # cannot reach it, so every wallet exports.
        target = self.tmp / "summer.csv"
        payload, code = _run(
            self.root, "transactions", "export", *_SCOPE, "--export-format", "cointracking-csv",
            "--from", "2025-06-01", "--to", "2025-07-31", "--file", str(target),
        )
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        self.assertEqual(payload["data"]["range"]["label"], "2025-06-01_2025-07-31")
        _, rows = _read_csv(target)
        self.assertEqual(sorted(row["Tx-ID"] for row in rows), ["pay-001", "sell-001"])
        workbook = self.tmp / "q1.xlsx"
        payload, code = _run(
            self.root, "transactions", "export", *_SCOPE, "--wallet", "Hot", "--period", "all",
            "--file", str(workbook),
        )
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        self.assertIsNone(payload["data"]["range"])
        payload, code = _run(
            self.root, "transactions", "export", *_SCOPE, "--export-format", "cointracking-csv",
            "--from", "2025-03-01", "--to", "2025-03-31", "--file", str(self.tmp / "march.csv"),
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "transaction_export_quarantined")

    def test_unknown_platform_is_rejected(self):
        from kassiber.core.transaction_formats import platform_format
        from kassiber.errors import AppError

        with self.assertRaises(AppError):
            platform_format("koinly")


_GENERIC_SCOPE = ("--workspace", "Main", "--profile", "Generic")


class TaxableConversionExportTests(unittest.TestCase):
    """A conversion booked as taxable is a sale and a purchase, not a transfer."""

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-taxable-conversion-")
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
        run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "Generic")
        for label, csv_text in (
            (
                "Onchain",
                "date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,description,kind\n"
                "2024-01-10T10:00:00Z,buy-1,inbound,BTC,1.00000000,0,40000,40000,Buy,buy\n"
                "2024-06-01T10:00:00Z,conv-out,outbound,BTC,0.10000000,0,,60000,Convert,transfer\n",
            ),
            (
                "Liquid",
                "date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,description,kind\n"
                "2024-06-01T10:10:00Z,conv-in,inbound,LBTC,0.10000000,0,,60000,Convert,transfer\n",
            ),
        ):
            path = tmp / f"{label}.csv"
            path.write_text(csv_text, encoding="utf-8")
            run("wallets", "create", *_GENERIC_SCOPE, "--label", label, "--kind", "custom")
            run("wallets", "import-csv", *_GENERIC_SCOPE, "--wallet", label, "--file", str(path))
        refs = {row["external_id"]: row["id"] for row in run("transactions", "list", *_GENERIC_SCOPE)}
        run(
            "transfers", "pair", *_GENERIC_SCOPE,
            "--tx-out", refs["conv-out"], "--tx-in", refs["conv-in"],
            "--kind", "peg-in", "--policy", "taxable",
        )
        run("journals", "process", *_GENERIC_SCOPE)

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def test_taxable_conversion_exports_as_sale_and_purchase_at_booked_values(self):
        target = self.tmp / "cointracking.csv"
        payload, code = _run(
            self.root, "transactions", "export", *_GENERIC_SCOPE,
            "--export-format", "cointracking-csv", "--file", str(target),
        )
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        _, rows = _read_csv(target)
        by_txid = {row["Tx-ID"]: row for row in rows}
        sale = by_txid["conv-out"]
        self.assertEqual(
            (sale["Type"], sale["Sell Amount"], sale["Sell Currency"], sale["Buy Amount"], sale["Buy Currency"]),
            ("Trade", "0.10000000", "BTC", "6000", "EUR"),
        )
        purchase = by_txid["conv-in"]
        self.assertEqual(
            (purchase["Type"], purchase["Buy Amount"], purchase["Buy Currency"], purchase["Sell Amount"], purchase["Sell Currency"]),
            ("Trade", "0.10000000", "LBTC", "6000", "EUR"),
        )
        self.assertTrue(any("taxable conversions" in warning for warning in payload["data"]["warnings"]))


class AltConversionExportTests(unittest.TestCase):
    """What the journal applied decides, not the policy asked for at pairing.

    Austrian Altbestand converted under the carrying-value policy still
    realizes a speculation gain, so the export must show a sale.
    """

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-alt-conversion-")
        tmp = Path(cls._tmp.name)
        cls.tmp = tmp
        cls.root = tmp / "data"

        def run(*args):
            payload, code = _run(cls.root, *args)
            if code != 0:
                raise AssertionError(f"{args} -> {json.dumps(payload)[:400]}")
            return payload["data"]

        scope = ("--workspace", "Main", "--profile", "Alt")
        run("init")
        run("workspaces", "create", "Main")
        run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", "at", "Alt")
        for label, csv_text in (
            (
                "Onchain",
                "date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,description,kind\n"
                "2021-01-10T10:00:00Z,buy-1,inbound,BTC,1.00000000,0,30000,30000,Buy,buy\n"
                "2021-02-01T10:00:00Z,conv-out,outbound,BTC,0.10000000,0,,50000,Convert,transfer\n",
            ),
            (
                "Liquid",
                "date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,description,kind\n"
                "2021-02-01T10:10:00Z,conv-in,inbound,LBTC,0.10000000,0,,50000,Convert,transfer\n",
            ),
        ):
            path = tmp / f"{label}.csv"
            path.write_text(csv_text, encoding="utf-8")
            run("wallets", "create", *scope, "--label", label, "--kind", "custom")
            run("wallets", "import-csv", *scope, "--wallet", label, "--file", str(path))
        refs = {row["external_id"]: row["id"] for row in run("transactions", "list", *scope)}
        run(
            "transfers", "pair", *scope,
            "--tx-out", refs["conv-out"], "--tx-in", refs["conv-in"],
            "--kind", "peg-in", "--policy", "carrying-value",
        )
        run("journals", "process", *scope)
        cls.scope = scope

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def test_a_realized_alt_conversion_exports_as_a_sale(self):
        target = self.tmp / "blockpit.csv"
        payload, code = _run(
            self.root, "transactions", "export", *self.scope,
            "--export-format", "blockpit-csv", "--file", str(target),
        )
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        _, rows = _read_csv(target)
        by_txid = {row["Trx. ID (optional)"]: row for row in rows}
        sale = by_txid["conv-out"]
        self.assertEqual(
            (sale["Label"], sale["Outgoing Asset"], sale["Outgoing Amount"], sale["Incoming Asset"]),
            ("Trade", "BTC", "0.10000000", "EUR"),
        )
        self.assertEqual(by_txid["conv-in"]["Label"], "Trade")
        self.assertEqual(by_txid["conv-in"]["Incoming Asset"], "LBTC")


class EventMappingTests(unittest.TestCase):
    def test_rows_that_would_change_their_tax_result_are_refused(self):
        from kassiber.core.transaction_formats import _event

        self.assertEqual(_event("internal_transfer+disposal", "outbound", "", False), (None, "mixed"))
        self.assertEqual(_event("internal_transfer+income", "inbound", "", False), (None, "mixed"))
        self.assertEqual(
            _event("disposal", "outbound", "", False, conversion=True, non_taxable=True),
            (None, "non_taxable_conversion"),
        )
        # Transfer-like parts only stay transfer legs, flagged for review.
        self.assertEqual(_event("internal_transfer+swap", "outbound", "", False), ("withdrawal", "swap"))

    def test_conversions_and_reviewed_taxability_keep_their_meaning(self):
        from kassiber.core.transaction_formats import BLOCKPIT, COINTRACKING, _event

        self.assertEqual(_event("disposal", "outbound", "", False, conversion=True), ("sell", "conversion"))
        self.assertEqual(_event("acquisition", "inbound", "", False, conversion=True), ("buy", "conversion"))
        self.assertEqual(_event("disposal", "outbound", "", False, non_taxable=True), ("payment_non_taxable", None))
        self.assertEqual(_event("income", "inbound", "", False, non_taxable=True), ("income_non_taxable", None))
        # Both tools have a non-taxable type, and Kassiber's importers read them back.
        from kassiber.importers import normalize_blockpit_record, normalize_cointracking_record

        cointracking = normalize_cointracking_record(
            {"Type": "Expense (non taxable)", "Sell Amount": "0.1", "Sell Currency": "BTC", "Date": "2025-01-01 10:00:00"}
        )
        self.assertEqual(cointracking["kind"], "expense_non_taxable")
        blockpit = normalize_blockpit_record(
            {"Date (UTC)": "01.01.2025 10:00:00", "Integration Name": "x", "Label": "Non-Taxable (Out)",
             "Outgoing Asset": "BTC", "Outgoing Amount": "0.1", "Incoming Asset": "", "Incoming Amount": ""}
        )
        self.assertEqual(blockpit["kind"], "expense_non_taxable")
        self.assertIn("payment_non_taxable", COINTRACKING.build_row.__globals__["_COINTRACKING_TYPES"])
        self.assertIn("payment_non_taxable", BLOCKPIT.build_row.__globals__["_BLOCKPIT_LABELS"])

    def test_only_journal_neutral_conversions_classify_as_swaps(self):
        from kassiber.core.transaction_classification import classification_entry_type, classify

        neutral = classification_entry_type("disposal", "neu_swap")
        self.assertEqual(neutral, "neutral_disposal")
        self.assertEqual(classification_entry_type("disposal", "alt_spekulation"), "disposal")
        self.assertEqual(classify({"disposal"}, set(), is_swap=False), "disposal")
        self.assertEqual(classify({neutral}, {"neu_swap"}, is_swap=True), "swap")
        self.assertEqual(classify({"acquisition"}, set(), is_swap=True), "swap")
        # A swap payout settled partly as a taxable disposal keeps both parts,
        # and no tool row can hold both.
        both = classify({neutral, "disposal"}, {"neu_swap"}, is_swap=True)
        self.assertEqual(both, "swap+disposal")
        from kassiber.core.transaction_formats import _event

        self.assertEqual(_event(both, "outbound", "", False), (None, "mixed"))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
