import tempfile
import unittest
from pathlib import Path

from kassiber.db import open_db


class MsatMigrationTests(unittest.TestCase):
    def test_legacy_transaction_rebuild_preserves_columns_added_after_real_schema(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-msat-transaction-") as tmp:
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
                """
            )
            conn.commit()
            conn.execute("PRAGMA foreign_keys = OFF")
            conn.executescript(
                """
                DROP TRIGGER IF EXISTS trg_custody_component_scope_insert;
                DROP TRIGGER IF EXISTS trg_custody_component_scope_update;
                DROP TRIGGER IF EXISTS trg_custody_gap_review_transaction_scope_insert;
                DROP TABLE transactions;
                CREATE TABLE transactions (
                    id TEXT PRIMARY KEY,
                    workspace_id TEXT NOT NULL,
                    profile_id TEXT NOT NULL,
                    wallet_id TEXT NOT NULL,
                    external_id TEXT,
                    fingerprint TEXT NOT NULL UNIQUE,
                    occurred_at TEXT NOT NULL,
                    confirmed_at TEXT,
                    direction TEXT NOT NULL,
                    asset TEXT NOT NULL,
                    amount REAL NOT NULL,
                    fee REAL NOT NULL DEFAULT 0,
                    fiat_currency TEXT,
                    fiat_rate REAL,
                    fiat_value REAL,
                    fiat_price_source TEXT,
                    fiat_rate_exact TEXT,
                    fiat_value_exact TEXT,
                    pricing_source_kind TEXT,
                    pricing_provider TEXT,
                    pricing_pair TEXT,
                    pricing_timestamp TEXT,
                    pricing_fetched_at TEXT,
                    pricing_granularity TEXT,
                    pricing_method TEXT,
                    pricing_external_ref TEXT,
                    pricing_quality TEXT,
                    commercial_applied_link_id TEXT,
                    review_status TEXT,
                    taxability_override INTEGER,
                    at_regime_override TEXT,
                    at_category_override TEXT,
                    privacy_boundary TEXT,
                    kind TEXT,
                    description TEXT,
                    counterparty TEXT,
                    note TEXT,
                    excluded INTEGER NOT NULL DEFAULT 0,
                    raw_json TEXT NOT NULL DEFAULT '{}',
                    created_at TEXT NOT NULL
                );
                INSERT INTO transactions VALUES(
                    'tx', 'ws', 'pf', 'wal', 'external', 'fingerprint',
                    '2024-01-02T00:00:00Z', '2024-01-02T00:01:00Z',
                    'outbound', 'BTC', 1.25, 0.00001,
                    'EUR', 40000, 50000, 'manual', '40000', '50000',
                    'manual', 'provider', 'BTC-EUR', '2024-01-02T00:00:00Z',
                    '2024-01-02T00:00:01Z', 'exact', 'override', 'ref', 'high',
                    'commercial-link', 'reviewed', 0, 'new', 'capital', 'coinjoin',
                    'payment', 'description', 'counterparty', 'note', 1, '{}',
                    '2024-01-02T00:00:00Z'
                );
                """
            )
            conn.commit()
            conn.close()

            conn = open_db(data_root)
            self.addCleanup(conn.close)
            row = conn.execute("SELECT * FROM transactions WHERE id = 'tx'").fetchone()

            self.assertEqual(row["amount"], 125_000_000_000)
            self.assertEqual(row["fee"], 1_000_000)
            self.assertEqual(row["commercial_applied_link_id"], "commercial-link")
            self.assertEqual(row["review_status"], "reviewed")
            self.assertEqual(row["taxability_override"], 0)
            self.assertEqual(row["at_regime_override"], "new")
            self.assertEqual(row["at_category_override"], "capital")
            self.assertEqual(row["privacy_boundary"], "coinjoin")

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
