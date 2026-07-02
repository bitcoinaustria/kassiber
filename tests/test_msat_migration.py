import tempfile
import unittest
from pathlib import Path

from kassiber.db import open_db


class MsatMigrationTests(unittest.TestCase):
    def test_legacy_derived_journal_quantities_migrate_to_msat(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-msat-migration-") as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(data_root)
            conn.executescript(
                """
                INSERT INTO workspaces(id, label, created_at)
                VALUES('ws', 'Main', '2024-01-01T00:00:00Z');
                INSERT INTO profiles(
                    id, workspace_id, label, fiat_currency, tax_country,
                    tax_long_term_days, gains_algorithm, created_at
                )
                VALUES(
                    'pf', 'ws', 'Default', 'EUR', 'generic', 365, 'FIFO',
                    '2024-01-01T00:00:00Z'
                );
                INSERT INTO accounts(
                    id, workspace_id, profile_id, code, label, account_type, asset, created_at
                )
                VALUES(
                    'acct', 'ws', 'pf', 'treasury', 'Treasury', 'asset', 'BTC',
                    '2024-01-01T00:00:00Z'
                );
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
                )
                VALUES('wal', 'ws', 'pf', 'acct', 'Cold', 'address', '{}', '2024-01-01T00:00:00Z');

                DROP TABLE journal_tax_summary;
                DROP TABLE journal_account_holdings;
                DROP TABLE journal_wallet_holdings;

                CREATE TABLE journal_tax_summary (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL,
                    profile_id TEXT NOT NULL,
                    year INTEGER NOT NULL,
                    asset TEXT NOT NULL,
                    transaction_type TEXT NOT NULL,
                    quantity REAL NOT NULL,
                    proceeds REAL NOT NULL DEFAULT 0,
                    cost_basis REAL NOT NULL DEFAULT 0,
                    gain_loss REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE journal_account_holdings (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL,
                    profile_id TEXT NOT NULL,
                    account_id TEXT,
                    account_code TEXT,
                    account_label TEXT,
                    asset TEXT NOT NULL,
                    quantity REAL NOT NULL,
                    cost_basis REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                CREATE TABLE journal_wallet_holdings (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL,
                    profile_id TEXT NOT NULL,
                    wallet_id TEXT,
                    wallet_label TEXT,
                    account_code TEXT,
                    asset TEXT NOT NULL,
                    quantity REAL NOT NULL,
                    cost_basis REAL NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );

                INSERT INTO journal_tax_summary VALUES(
                    'tax', 'ws', 'pf', 2024, 'BTC', 'sell',
                    0.5, 20000, 10000, 10000,
                    '2024-01-01T00:00:00Z'
                );
                INSERT INTO journal_account_holdings VALUES(
                    'acct-h', 'ws', 'pf', 'acct', 'treasury', 'Treasury', 'BTC', 1.25, 25000,
                    '2024-01-01T00:00:00Z'
                );
                INSERT INTO journal_wallet_holdings VALUES(
                    'wallet-h', 'ws', 'pf', 'wal', 'Cold', 'treasury', 'BTC', 0.75, 15000,
                    '2024-01-01T00:00:00Z'
                );
                """
            )
            conn.commit()
            conn.close()

            conn = open_db(data_root)
            self.addCleanup(conn.close)

            self.assertEqual(
                conn.execute(
                    "SELECT quantity FROM journal_tax_summary WHERE id = 'tax'"
                ).fetchone()[0],
                50_000_000_000,
            )
            self.assertEqual(
                conn.execute(
                    "SELECT quantity FROM journal_account_holdings WHERE id = 'acct-h'"
                ).fetchone()[0],
                125_000_000_000,
            )
            self.assertEqual(
                conn.execute(
                    "SELECT quantity FROM journal_wallet_holdings WHERE id = 'wallet-h'"
                ).fetchone()[0],
                75_000_000_000,
            )

            for table in (
                "journal_tax_summary",
                "journal_account_holdings",
                "journal_wallet_holdings",
            ):
                quantity_column = next(
                    row
                    for row in conn.execute(f"PRAGMA table_info({table})")
                    if row["name"] == "quantity"
                )
                self.assertEqual(quantity_column["type"], "INTEGER")

            self.assertIsNotNone(
                conn.execute(
                    """
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'index' AND name = 'idx_journal_tax_summary_profile_year'
                    """
                ).fetchone()
            )


if __name__ == "__main__":
    unittest.main()
