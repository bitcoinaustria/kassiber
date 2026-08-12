import sqlite3
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path
from unittest.mock import patch

from kassiber import importers
from kassiber.cli import handlers as cli_handlers
from kassiber.core import attachments, commercial, import_batches, imports as core_imports
from kassiber.core import metadata as core_metadata
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

    @staticmethod
    def _hooks(invalidations):
        def ensure_tag_row(conn, workspace_id, profile_id, code, label):
            existing = conn.execute(
                "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
                (profile_id, code),
            ).fetchone()
            if existing:
                return existing, False
            conn.execute(
                "INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at) "
                "VALUES(?, ?, ?, ?, ?, ?)",
                (f"{profile_id}-{code}", workspace_id, profile_id, code, label, now_iso()),
            )
            return conn.execute(
                "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
                (profile_id, code),
            ).fetchone(), True

        return core_imports.ImportCoordinatorHooks(
            ensure_tag_row=ensure_tag_row,
            invalidate_journals=lambda _conn, profile_id: invalidations.append(profile_id),
        )

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

    def test_idless_provider_collisions_survive_repeat_and_later_txid_upgrade(self):
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
            provider_wallet = conn.execute(
                "SELECT * FROM wallets WHERE id = 'cointracking'"
            ).fetchone()
            descriptor_wallet = conn.execute(
                "SELECT * FROM wallets WHERE id = 'descriptor'"
            ).fetchone()
            body = """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Deposit,0.01000000,BTC,,,,,Wallet,same source facts,2024-06-01T12:00:00Z,
Deposit,0.01000000,BTC,,,,,Wallet,same source facts,2024-06-01T12:00:00Z,
"""
            source = self._write(tmp, "cointracking.csv", body)
            first = core_imports.import_file_into_wallet(
                conn,
                profile,
                provider_wallet,
                str(source),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(first["imported"], 2)
            self.assertEqual(first["ambiguous"], 2)
            self.assertEqual(first["excluded"], 2)
            provider_rows = conn.execute(
                "SELECT id, fingerprint FROM transactions WHERE wallet_id = 'cointracking'"
            ).fetchall()
            self.assertEqual(len(provider_rows), 2)
            self.assertEqual(len({row["fingerprint"] for row in provider_rows}), 2)

            retry = core_imports.import_file_into_wallet(
                conn,
                profile,
                provider_wallet,
                str(source),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(retry["imported"], 0)
            self.assertEqual(retry["skipped"], 2)
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM transactions WHERE wallet_id = 'cointracking'"
                ).fetchone()[0],
                2,
            )

            metadata_update = self._write(
                tmp,
                "cointracking-updated-comment.csv",
                body.replace("same source facts", "updated source comment"),
            )
            changed = core_imports.import_file_into_wallet(
                conn,
                profile,
                provider_wallet,
                str(metadata_update),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(changed["imported"], 0)
            self.assertEqual(changed["skipped"], 2)
            self.assertEqual(len(changed["updated_records"]), 0)
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM transactions WHERE wallet_id = 'cointracking'"
                ).fetchone()[0],
                2,
            )

            txid = "12" * 32
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
                    }
                ],
                "descriptor-sync",
                self._hooks([]),
                authoritative_chain_observer=True,
            )
            richer = self._write(
                tmp,
                "cointracking-with-id.csv",
                body.replace(
                    "2024-06-01T12:00:00Z,\nDeposit",
                    f"2024-06-01T12:00:00Z,{txid}\nDeposit",
                    1,
                ),
            )
            upgraded = core_imports.import_file_into_wallet(
                conn,
                profile,
                provider_wallet,
                str(richer),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(len(upgraded["updated_records"]), 1)
            self.assertEqual(upgraded["matched"], 1)
            rows = conn.execute(
                """
                SELECT t.external_id, t.external_id_kind, tags.code
                FROM transactions t
                JOIN transaction_tags tt ON tt.transaction_id = t.id
                JOIN tags ON tags.id = tt.tag_id
                WHERE t.wallet_id = 'cointracking'
                ORDER BY t.external_id IS NULL, t.id
                """
            ).fetchall()
            self.assertEqual(len(rows), 2)
            self.assertEqual(
                (rows[0]["external_id"], rows[0]["external_id_kind"], rows[0]["code"]),
                (txid, "txid", "cointracking-matched"),
            )
            self.assertEqual(rows[1]["code"], "cointracking-ambiguous")

            duplicate_txid = "34" * 32
            duplicate_source = self._write(
                tmp,
                "cointracking-duplicate-txid.csv",
                """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Deposit,0.03000000,BTC,,,,,Wallet,duplicate id,2024-06-03T12:00:00Z,"""
                + duplicate_txid
                + "\nDeposit,0.03000000,BTC,,,,,Wallet,duplicate id,2024-06-03T12:00:00Z,"
                + duplicate_txid
                + "\n",
            )
            duplicate_first = core_imports.import_file_into_wallet(
                conn,
                profile,
                provider_wallet,
                str(duplicate_source),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(duplicate_first["imported"], 1)
            self.assertEqual(duplicate_first["skipped"], 1)
            duplicate_retry = core_imports.import_file_into_wallet(
                conn,
                profile,
                provider_wallet,
                str(duplicate_source),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(duplicate_retry["imported"], 0)
            self.assertEqual(duplicate_retry["skipped"], 2)
            self.assertEqual(duplicate_retry["updated_records"], [])
            self.assertFalse(duplicate_retry["journal_invalidated"])
            conn.close()

    def test_provider_history_reconciles_overlap_with_descriptor_wallet(self):
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
                ("blockpit", "Blockpit history", "blockpit"),
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
            blockpit_wallet = conn.execute(
                "SELECT * FROM wallets WHERE id = 'blockpit'"
            ).fetchone()
            invalidations = []
            hooks = self._hooks(invalidations)
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
                    },
                    {
                        "txid": "cd" * 32,
                        "occurred_at": "2024-06-02T12:00:00Z",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": Decimal("0.03"),
                        "fee": Decimal("0"),
                        "kind": "deposit",
                    },
                ],
                "descriptor-sync",
                hooks,
            )
            body = f"""Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Deposit,0.01000000,BTC,,,,,Wallet,duplicate,2024-06-01T12:00:00Z,{txid}
Deposit,0.03000000,BTC,,,,,Wallet,needs review,2024-06-02T12:00:00Z,
Deposit,0.02000000,BTC,,,,,Wallet,provider only,2024-06-03T12:00:00Z,ct-only
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
            outcome = core_imports.import_file_into_wallet(
                conn,
                profile,
                migration_wallet,
                str(source),
                "cointracking_csv",
                hooks,
            )
            self.assertEqual(outcome["matched"], 1)
            self.assertEqual(outcome["ambiguous"], 1)
            self.assertEqual(outcome["unmatched"], 1)
            self.assertEqual(outcome["excluded"], 2)
            self.assertEqual(outcome["reconciliation_changed"], 2)
            rows = conn.execute(
                """
                SELECT t.occurred_at, t.excluded, GROUP_CONCAT(tags.code) AS tags
                FROM transactions t
                LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
                LEFT JOIN tags ON tags.id = tt.tag_id
                WHERE t.wallet_id = 'cointracking'
                GROUP BY t.id
                ORDER BY t.occurred_at
                """
            ).fetchall()
            self.assertEqual(
                [(row["excluded"], row["tags"]) for row in rows],
                [
                    (1, "cointracking-matched"),
                    (1, "cointracking-ambiguous"),
                    (0, None),
                ],
            )
            invalidations_before_retry = len(invalidations)
            retry = core_imports.import_file_into_wallet(
                conn,
                profile,
                migration_wallet,
                str(source),
                "cointracking_csv",
                hooks,
            )
            self.assertEqual(retry["reconciliation_changed"], 0)
            self.assertEqual(len(invalidations), invalidations_before_retry)
            blockpit_source = self._write(
                tmp,
                "blockpit.csv",
                """Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID
2024-06-01T12:00:00Z,Cold wallet,Unlabeled Deposit,,,BTC,0.01000000,,,duplicate,"""
                + txid
                + "\n",
            )
            blockpit = core_imports.import_file_into_wallet(
                conn,
                profile,
                blockpit_wallet,
                str(blockpit_source),
                "blockpit_csv",
                hooks,
            )
            self.assertEqual(blockpit["matched"], 1)
            self.assertEqual(blockpit["excluded"], 1)
            blockpit_tag = conn.execute(
                """
                SELECT tags.code
                FROM transactions t
                JOIN transaction_tags tt ON tt.transaction_id = t.id
                JOIN tags ON tags.id = tt.tag_id
                WHERE t.wallet_id = 'blockpit'
                """
            ).fetchone()
            self.assertEqual(blockpit_tag["code"], "blockpit-matched")
            rolled_back = import_batches.rollback_batch(
                conn,
                profile,
                outcome["import_batch_id"],
                invalidate_journals=hooks.invalidate_journals,
            )
            self.assertEqual(rolled_back["transactions_deleted"], 3)
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM transactions WHERE wallet_id = 'cointracking'"
                ).fetchone()[0],
                0,
            )
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM transactions WHERE wallet_id = 'descriptor'"
                ).fetchone()[0],
                2,
            )
            conn.close()

    def test_descriptor_sync_reconciles_provider_history_imported_first(self):
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
            txid = "ef" * 32
            second_txid = "fe" * 32
            source = self._write(
                tmp,
                "cointracking.csv",
                """Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Comment,Date,Tx-ID (optional)
Deposit,0.01000000,BTC,,,,,Wallet,duplicate later,2024-06-01T12:00:00Z,"""
                + txid
                + "\nDeposit,0.02000000,BTC,,,,,Wallet,second duplicate later,2024-06-02T12:00:00Z,"
                + second_txid
                + "\n",
            )
            imported = core_imports.import_file_into_wallet(
                conn,
                profile,
                migration_wallet,
                str(source),
                "cointracking_csv",
                self._hooks([]),
            )
            self.assertEqual(imported["unmatched"], 2)
            self.assertEqual(imported["excluded"], 0)

            def sync_with_observation(
                sync_conn,
                _runtime_config,
                sync_profile,
                wallets,
                sync_hooks,
                **_kwargs,
            ):
                outcome = sync_hooks.insert_records(
                    sync_conn,
                    sync_profile,
                    wallets[0],
                    [
                        {
                            "txid": txid,
                            "occurred_at": "2024-06-01T12:00:00Z",
                            "direction": "inbound",
                            "asset": "BTC",
                            "amount": Decimal("0.01"),
                            "fee": Decimal("0"),
                            "kind": "deposit",
                        },
                        {
                            "txid": second_txid,
                            "occurred_at": "2024-06-02T12:00:00Z",
                            "direction": "inbound",
                            "asset": "BTC",
                            "amount": Decimal("0.02"),
                            "fee": Decimal("0"),
                            "kind": "deposit",
                        },
                    ],
                    "backend:test",
                    authoritative_chain_observer=True,
                )
                outcome.pop("_observer_resolved_records", None)
                return [{**outcome, "wallet": wallets[0]["label"], "status": "synced"}]

            with patch.object(
                cli_handlers.core_sync,
                "sync_wallets",
                side_effect=sync_with_observation,
            ):
                results = cli_handlers._apply_wallet_sync_atomically(
                    conn,
                    {},
                    profile,
                    descriptor_wallet,
                    cli_handlers._wallet_sync_hooks(commit=False),
                    prefetched={},
                )
                repeat = cli_handlers._apply_wallet_sync_atomically(
                    conn,
                    {},
                    profile,
                    descriptor_wallet,
                    cli_handlers._wallet_sync_hooks(commit=False),
                    prefetched={},
                )
            self.assertEqual(results[0]["matched"], 2)
            self.assertEqual(results[0]["excluded"], 2)
            self.assertEqual(results[0]["reconciliation_changed"], 2)
            self.assertEqual(repeat[0]["reconciliation_changed"], 0)
            provider_rows = conn.execute(
                "SELECT id, external_id, excluded FROM transactions "
                "WHERE wallet_id = 'cointracking' ORDER BY occurred_at"
            ).fetchall()
            provider_row = provider_rows[0]
            tag = conn.execute(
                """
                SELECT tags.code
                FROM transaction_tags tt
                JOIN tags ON tags.id = tt.tag_id
                WHERE tt.transaction_id = ?
                """,
                (provider_row["id"],),
            ).fetchone()
            self.assertEqual(provider_row["excluded"], 1)
            self.assertEqual(tag["code"], "cointracking-matched")

            core_metadata.set_transaction_excluded(
                conn,
                "ws",
                "prof",
                provider_row["id"],
                False,
                cli_handlers._metadata_hooks(),
                source="gui",
            )
            manual_inclusion = core_imports.reconcile_tax_platform_history(
                conn,
                profile,
                cli_handlers._import_coordinator_hooks(),
            )
            self.assertEqual(manual_inclusion["reconciliation_changed"], 0)
            self.assertEqual(manual_inclusion["excluded"], 1)
            self.assertEqual(
                conn.execute(
                    "SELECT excluded FROM transactions WHERE id = ?",
                    (provider_row["id"],),
                ).fetchone()[0],
                0,
            )
            conn.execute(
                "UPDATE transactions SET excluded = 1 WHERE wallet_id = 'descriptor'"
            )
            excluded_authority = core_imports.reconcile_tax_platform_history(
                conn,
                profile,
                cli_handlers._import_coordinator_hooks(),
            )
            self.assertEqual(excluded_authority["matched"], 2)
            self.assertEqual(excluded_authority["reconciliation_changed"], 0)
            self.assertEqual(
                conn.execute(
                    "SELECT excluded FROM transactions WHERE id = ?",
                    (provider_row["id"],),
                ).fetchone()[0],
                0,
            )

            core_metadata.set_transaction_excluded(
                conn,
                "ws",
                "prof",
                provider_row["id"],
                True,
                cli_handlers._metadata_hooks(),
                source="gui",
            )

            conn.execute("DELETE FROM transactions WHERE wallet_id = 'descriptor'")
            retracted = core_imports.reconcile_tax_platform_history(
                conn,
                profile,
                cli_handlers._import_coordinator_hooks(),
            )
            self.assertEqual(retracted["unmatched"], 2)
            self.assertEqual(retracted["reactivated"], 1)
            self.assertEqual(
                conn.execute(
                    "SELECT excluded FROM transactions WHERE id = ?",
                    (provider_row["id"],),
                ).fetchone()[0],
                1,
            )
            self.assertEqual(
                conn.execute(
                    "SELECT excluded FROM transactions WHERE id = ?",
                    (provider_rows[1]["id"],),
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
