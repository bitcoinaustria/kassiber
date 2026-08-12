import sqlite3
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from kassiber import importers
from kassiber.core import attachments, commercial, imports as core_imports
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import now_iso


COINTRACKING_CSV = """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Trade-Group,Comment,Date,Tx-ID (optional)
Trade,0.01000000,BTC,500.00,EUR,2.00,EUR,Kraken,savings,first buy,01.02.2024 12:00:00,ct-buy
Trade,550.00,EUR,0.01000000,BTC,0.00001000,BTC,Kraken,,sale,2024-03-01T12:00:00Z,ct-sell
Staking,0.00010000,BTC,,,,,Wallet,,reward,2024-04-01T12:00:00Z,ct-stake
Trade,1.0,ETH,3000.00,EUR,0,,Kraken,,altcoin only,2024-05-01T12:00:00Z,ct-eth
"""

BLOCKPIT_CSV = """Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID,Source Type,Source Name
2024-01-01T10:00:00Z,Kraken,Trade,EUR,500.00,BTC,0.01000000,EUR,2.00,buy,bp-buy,API,Kraken API
2024-02-01T10:00:00Z,Cold wallet,Payment,BTC,0.00100000,,,BTC,0.00001000,coffee,bp-pay,Chain,Bitcoin
2024-03-01T10:00:00Z,Cold wallet,Unlabeled Deposit,,,BTC,0.02000000,,,receive,bp-in,Chain,Bitcoin
2024-04-01T10:00:00Z,Exchange,Trade,EUR,1000.00,ETH,0.5,,,altcoin,bp-eth,API,Exchange API
"""


class TaxPlatformParserTests(unittest.TestCase):
    def _write(self, directory, name, body):
        path = Path(directory) / name
        path.write_text(body, encoding="utf-8")
        return path

    def test_cointracking_full_history_keeps_only_bitcoin_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            rows = importers.load_cointracking_csv_records(
                self._write(tmp, "cointracking.csv", COINTRACKING_CSV)
            )
        self.assertEqual([row["txid"] for row in rows], ["ct-buy", "ct-sell", "ct-stake"])
        self.assertEqual([row["kind"] for row in rows], ["buy", "sell", "staking"])
        self.assertEqual(rows[0]["fiat_value"], 502)
        self.assertEqual(rows[1]["fee"], Decimal("0.00001000"))
        self.assertEqual(rows[0]["pricing_method"], "cointracking_csv")

    def test_blockpit_full_history_maps_documented_labels(self):
        with tempfile.TemporaryDirectory() as tmp:
            rows = importers.load_blockpit_csv_records(
                self._write(tmp, "blockpit.csv", BLOCKPIT_CSV)
            )
        self.assertEqual([row["txid"] for row in rows], ["bp-buy", "bp-pay", "bp-in"])
        self.assertEqual([row["kind"] for row in rows], ["buy", "spend", "deposit"])
        self.assertEqual(rows[0]["fiat_value"], 502)
        self.assertEqual(rows[0]["pricing_method"], "blockpit_csv")

    def test_ambiguous_platform_types_fail_closed(self):
        body = COINTRACKING_CSV.replace("Staking,", "Receive Loan,")
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(AppError) as raised:
                importers.load_cointracking_csv_records(
                    self._write(tmp, "cointracking.csv", body)
                )
        self.assertIn("needs review", str(raised.exception))

    def test_cointracking_cross_asset_account_value_fails_without_currency(self):
        body = """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional),Buy Value in Account Currency (optional),Sell Value in Account Currency (optional)
Trade,0.01000000,BTC,1.0,ETH,,,Kraken,btc for eth,2024-06-01T12:00:00Z,ct-cross,1234.56,
"""
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(AppError) as raised:
                importers.load_cointracking_csv_records(
                    self._write(tmp, "cointracking.csv", body)
                )
        self.assertIn("unsupported asset ETH", str(raised.exception))
        self.assertIn("explicit fiat currency", raised.exception.hint)

    def test_blockpit_standalone_bitcoin_fee_is_an_outbound_row(self):
        body = """Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID
2024-06-01T12:00:00Z,Kraken,Fee,,,,,BTC,0.00002000,withdrawal fee,bp-fee
"""
        with tempfile.TemporaryDirectory() as tmp:
            rows = importers.load_blockpit_csv_records(
                self._write(tmp, "blockpit.csv", body)
            )
        self.assertEqual(rows[0]["direction"], "outbound")
        self.assertEqual(rows[0]["amount"], Decimal("0.00002000"))
        self.assertEqual(rows[0]["fee"], Decimal("0"))

    def test_third_asset_fee_fails_closed(self):
        body = """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Trade,0.01000000,BTC,500.00,EUR,2.00,USDT,Kraken,unsupported fee,2024-06-01T12:00:00Z,ct-third-fee
"""
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(AppError) as raised:
                importers.load_cointracking_csv_records(
                    self._write(tmp, "cointracking.csv", body)
                )
        self.assertIn("unsupported asset USDT", str(raised.exception))

    def test_cointracking_simple_fee_and_non_taxable_types_import(self):
        body = """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Income (non taxable),0.001,BTC,,,,,Wallet,non-taxable receipt,2024-07-01T12:00:00Z,ct-nt-income
Other Income (non taxable),0.002,BTC,,,,,Wallet,other receipt,2024-07-02T12:00:00Z,ct-nt-other-income
Airdrop (non taxable),0.003,BTC,,,,,Wallet,airdrop receipt,2024-07-03T12:00:00Z,ct-nt-airdrop
Other Fee,,,,,0.0001,BTC,Wallet,standalone fee,2024-07-04T12:00:00Z,ct-fee
Other Expense,,,0.0002,BTC,,,Wallet,ordinary expense,2024-07-05T12:00:00Z,ct-expense
Expense (non taxable),,,0.0003,BTC,,,Wallet,non-taxable outflow,2024-07-06T12:00:00Z,ct-nt-expense
"""
        with tempfile.TemporaryDirectory() as tmp:
            rows = importers.load_cointracking_csv_records(
                self._write(tmp, "cointracking.csv", body)
            )
        self.assertEqual(len(rows), 6)
        self.assertEqual([row["kind"] for row in rows[:3]], [None, None, None])
        self.assertEqual(rows[3]["kind"], "spend")
        self.assertEqual(rows[3]["amount"], Decimal("0.0001"))
        self.assertEqual(rows[3]["fee"], Decimal("0"))
        self.assertEqual(rows[4]["kind"], "spend")
        self.assertEqual(rows[5]["kind"], "expense_non_taxable")

    def test_provider_history_rejects_overlap_with_descriptor_wallet(self):
        with tempfile.TemporaryDirectory() as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            conn.row_factory = sqlite3.Row
            now = now_iso()
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Default', ?)",
                (now,),
            )
            conn.execute(
                "INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at) "
                "VALUES('prof', 'ws', 'Book', 'EUR', ?)",
                (now,),
            )
            for wallet_id, label, kind in (
                ("descriptor", "Cold storage", "descriptor"),
                ("cointracking", "CoinTracking history", "cointracking"),
            ):
                conn.execute(
                    "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at) "
                    "VALUES(?, 'ws', 'prof', ?, ?, ?)",
                    (wallet_id, label, kind, now),
                )
            profile = conn.execute("SELECT * FROM profiles WHERE id = 'prof'").fetchone()
            descriptor_wallet = conn.execute(
                "SELECT * FROM wallets WHERE id = 'descriptor'"
            ).fetchone()
            migration_wallet = conn.execute(
                "SELECT * FROM wallets WHERE id = 'cointracking'"
            ).fetchone()
            hooks = core_imports.ImportCoordinatorHooks(
                ensure_tag_row=lambda *_args: ({"id": "unused"}, False),
                invalidate_journals=lambda *_args: None,
            )
            txid = "ab" * 32
            core_imports.import_records_into_wallet(
                conn,
                profile,
                descriptor_wallet,
                [
                    {
                        "txid": txid,
                        "occurred_at": "2024-06-01T12:00:00Z",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": Decimal("0.01"),
                        "fee": Decimal("0"),
                        "kind": "deposit",
                    }
                ],
                "descriptor-sync",
                hooks,
            )
            body = f"""Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Deposit,0.01000000,BTC,,,,,Wallet,duplicate,2024-06-01T12:00:00Z,{txid}
"""
            source = self._write(tmp, "cointracking.csv", body)
            for source_format in ("cointracking_csv", "blockpit_csv"):
                with self.subTest(source_format=source_format):
                    with self.assertRaises(AppError) as wrong_wallet:
                        core_imports.import_file_into_wallet(
                            conn,
                            profile,
                            descriptor_wallet,
                            str(source),
                            source_format,
                            hooks,
                        )
                    self.assertEqual(
                        wrong_wallet.exception.code,
                        "migration_wallet_kind_required",
                    )
            with self.assertRaises(AppError) as raised:
                core_imports.import_file_into_wallet(
                    conn,
                    profile,
                    migration_wallet,
                    str(source),
                    "cointracking_csv",
                    hooks,
                )
            self.assertEqual(raised.exception.code, "migration_source_overlap")
            self.assertEqual(
                raised.exception.details["conflicts"][0]["wallets"],
                ["Cold storage"],
            )
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM transactions WHERE wallet_id = 'cointracking'"
                ).fetchone()[0],
                0,
            )
            conn.close()


class PriorTaxReportArchiveTests(unittest.TestCase):
    def test_prior_report_is_copied_and_marked_as_evidence_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp) / "data"
            report = Path(tmp) / "cointracking-2024.pdf"
            report.write_bytes(b"%PDF-1.4\nprior report\n")
            conn = open_db(str(data_root))
            conn.row_factory = sqlite3.Row
            now = now_iso()
            conn.execute(
                "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Default', ?)",
                (now,),
            )
            conn.execute(
                "INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at) VALUES('prof', 'ws', 'Book', 'EUR', ?)",
                (now,),
            )

            def resolve_scope(_conn, _workspace=None, _profile=None):
                return (
                    _conn.execute("SELECT * FROM workspaces WHERE id = 'ws'").fetchone(),
                    _conn.execute("SELECT * FROM profiles WHERE id = 'prof'").fetchone(),
                )

            hooks = commercial.CommercialHooks(
                resolve_scope=resolve_scope,
                resolve_transaction=lambda *_args: None,
                invalidate_journals=lambda *_args: None,
            )
            result = commercial.import_prior_tax_report(
                conn,
                data_root,
                None,
                None,
                hooks,
                provider="CoinTracking",
                file_path=report,
                tax_year="2024",
            )
            document = result["document"]
            self.assertEqual(document["document_type"], "tax_report")
            self.assertEqual(document["label"], "CoinTracking 2024 tax report")
            self.assertEqual(document["issuer"], "CoinTracking")
            self.assertIn("evidence only", document["notes"])
            attachment = conn.execute(
                "SELECT * FROM attachments WHERE id = ?",
                (result["attachment"]["attachment_id"],),
            ).fetchone()
            self.assertTrue(
                (attachments._attachments_root(str(data_root)) / attachment["stored_relpath"]).is_file()
            )
            conn.close()


if __name__ == "__main__":
    unittest.main()
