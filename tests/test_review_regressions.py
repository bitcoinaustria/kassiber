import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


_OLD_SCHEMA_SQL = """
PRAGMA foreign_keys = ON;

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    fiat_currency TEXT NOT NULL DEFAULT 'USD',
    tax_country TEXT NOT NULL DEFAULT 'generic',
    tax_long_term_days INTEGER NOT NULL DEFAULT 365,
    gains_algorithm TEXT NOT NULL DEFAULT 'FIFO',
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (workspace_id, label)
);

CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    account_type TEXT NOT NULL,
    asset TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE wallets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, label)
);

CREATE TABLE transactions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    external_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount REAL NOT NULL,
    fee REAL NOT NULL DEFAULT 0,
    fiat_currency TEXT,
    fiat_rate REAL,
    fiat_value REAL,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (transaction_id, tag_id)
);

CREATE TABLE journal_entries (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    occurred_at TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    quantity REAL NOT NULL,
    fiat_value REAL NOT NULL DEFAULT 0,
    unit_cost REAL NOT NULL DEFAULT 0,
    cost_basis REAL,
    proceeds REAL,
    gain_loss REAL,
    description TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE journal_quarantines (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE bip329_labels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    record_type TEXT NOT NULL,
    ref TEXT NOT NULL,
    label TEXT,
    origin TEXT,
    spendable INTEGER,
    data_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
"""


class ReviewRegressionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-review-regressions-")
        cls.tmp_path = Path(cls._tmp.name)

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def setUp(self):
        self.case_dir = self.tmp_path / self.id().split(".")[-1]
        self.case_dir.mkdir(parents=True, exist_ok=True)
        self.data_root = self.case_dir / "data"

    def _run_cli(self, *args, machine=False, output=None):
        cmd = [sys.executable, "-m", "kassiber", "--data-root", str(self.data_root)]
        if machine:
            cmd.append("--machine")
        if output is not None:
            cmd.extend(["--output", str(output)])
        cmd.extend(args)
        return subprocess.run(
            cmd,
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _run_json(self, *args):
        result = self._run_cli(*args, machine=True)
        stdout = result.stdout.strip()
        self.assertTrue(stdout, msg=f"No stdout for {args!r}; stderr={result.stderr!r}")
        try:
            payload = json.loads(stdout)
        except json.JSONDecodeError as exc:
            self.fail(f"stdout was not JSON for {args!r}: {stdout[:400]}")
        self.assertEqual(payload.get("schema_version"), 1)
        return payload, result

    def _assert_ok(self, payload, result, kind):
        self.assertEqual(result.returncode, 0, msg=f"{payload!r}")
        self.assertEqual(payload.get("kind"), kind)

    def _bootstrap_wallet(self, label="Wallet", kind="phoenix"):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json("profiles", "create", "--workspace", "Main", "Default")
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", label,
            "--kind", kind,
        )
        self._assert_ok(payload, result, "wallets.create")

    def _insert_transaction(self, *, wallet_label, tx_id, occurred_at, amount_msat, direction="inbound", asset="BTC"):
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        profile = conn.execute(
            "SELECT id, workspace_id, fiat_currency FROM profiles WHERE label = 'Default'"
        ).fetchone()
        wallet = conn.execute(
            "SELECT id FROM wallets WHERE label = ?",
            (wallet_label,),
        ).fetchone()
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, counterparty, note,
                excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                tx_id,
                f"fp-{tx_id}",
                occurred_at,
                direction,
                asset,
                amount_msat,
                0,
                profile["fiat_currency"],
                None,
                None,
                "deposit",
                tx_id,
                None,
                None,
                0,
                "{}",
                occurred_at,
            ),
        )
        conn.commit()
        conn.close()

    def test_btcpay_import_machine_mode_keeps_json_envelope(self):
        self._bootstrap_wallet(label="BTCPay")
        btcpay_csv = self.case_dir / "btcpay.csv"
        btcpay_csv.write_text(
            "TransactionId,Timestamp,Currency,Amount,Comment,Labels\n"
            "tx-1,2024-01-01T00:00:00Z,BTC,0.001 BTC,seeded,merchant\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTCPay",
            "--file", str(btcpay_csv),
            "--format", "csv",
        )
        self._assert_ok(payload, result, "wallets.import-btcpay")
        self.assertEqual(payload["data"]["input_format"], "btcpay_csv")
        self.assertEqual(payload["data"]["imported"], 1)

    def test_transactions_list_returns_btc_and_msat_fields(self):
        self._bootstrap_wallet(label="One")
        json_file = self.case_dir / "import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "demo",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "One",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(len(payload["data"]), 1)
        record = payload["data"][0]
        self.assertAlmostEqual(record["amount"], 0.001, places=12)
        self.assertEqual(record["amount_msat"], 100_000_000)
        self.assertEqual(record["fee_msat"], 0)

    def test_rates_latest_prefers_manual_override_at_same_timestamp(self):
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000", "--source", "coingecko"
        )
        self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "65000", "--source", "manual"
        )
        self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json("rates", "latest", "BTC-USD")
        self._assert_ok(payload, result, "rates.latest")
        self.assertEqual(payload["data"]["source"], "manual")
        self.assertAlmostEqual(payload["data"]["rate"], 65000.0, places=4)

    def test_journals_process_autopriced_from_cached_rate(self):
        self._bootstrap_wallet(label="CacheA")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CacheA",
            tx_id="cache-a",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-a'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 600.0, places=4)

    def test_journals_process_misses_future_only_rate(self):
        self._bootstrap_wallet(label="CacheFuture")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-02T00:00:00Z", "70000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CacheFuture",
            tx_id="cache-future",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 0)
        self.assertEqual(payload["data"]["quarantined"], 1)
        self.assertEqual(payload["data"]["auto_priced"], 0)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-future'"
        ).fetchone()
        quarantine = conn.execute(
            "SELECT reason FROM journal_quarantines WHERE transaction_id = (SELECT id FROM transactions WHERE external_id = 'cache-future')"
        ).fetchone()
        conn.close()
        self.assertIsNone(tx["fiat_rate"])
        self.assertIsNone(tx["fiat_value"])
        self.assertEqual(quarantine["reason"], "missing_spot_price")

    def test_journals_process_prefers_manual_rate_same_timestamp(self):
        self._bootstrap_wallet(label="CacheManual")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000", "--source", "coingecko"
        )
        self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "65000", "--source", "manual"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CacheManual",
            tx_id="cache-manual",
            occurred_at="2024-05-01T00:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-manual'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 65000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 650.0, places=4)

    def test_journals_process_autoprices_lbtc_from_btc_rate(self):
        self._bootstrap_wallet(label="LiquidLike")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="LiquidLike",
            tx_id="cache-lbtc",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
            asset="LBTC",
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-lbtc'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 600.0, places=4)

    def test_balance_history_uses_historical_rate_and_remaining_basis(self):
        self._bootstrap_wallet(label="W")
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        profile = conn.execute("SELECT id, workspace_id FROM profiles WHERE label = 'Default'").fetchone()
        wallet = conn.execute("SELECT id, account_id FROM wallets WHERE label = 'W'").fetchone()
        conn.executescript(
            f"""
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, raw_json, created_at
            ) VALUES(
                'tx1', '{profile["workspace_id"]}', '{profile["id"]}', '{wallet["id"]}', 'buy', 'fp1',
                '2024-01-10T00:00:00Z', 'inbound', 'BTC', 100000000000, 0, 'USD',
                10000, 10000, 'deposit', 'buy', '{{}}', '2024-01-10T00:00:00Z'
            );
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, raw_json, created_at
            ) VALUES(
                'tx2', '{profile["workspace_id"]}', '{profile["id"]}', '{wallet["id"]}', 'sell', 'fp2',
                '2024-02-10T00:00:00Z', 'outbound', 'BTC', 50000000000, 0, 'USD',
                20000, 10000, 'withdrawal', 'sell', '{{}}', '2024-02-10T00:00:00Z'
            );
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(
                'je1', '{profile["workspace_id"]}', '{profile["id"]}', 'tx1', '{wallet["id"]}', '{wallet["account_id"]}',
                '2024-01-10T00:00:00Z', 'acquisition', 'BTC', 100000000000, 10000, 10000,
                NULL, NULL, NULL, 'buy', '2024-01-10T00:00:00Z'
            );
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(
                'je2', '{profile["workspace_id"]}', '{profile["id"]}', 'tx2', '{wallet["id"]}', '{wallet["account_id"]}',
                '2024-02-10T00:00:00Z', 'disposal', 'BTC', -50000000000, 10000, 0,
                5000, 10000, 5000, 'sell', '2024-02-10T00:00:00Z'
            );
            UPDATE profiles
            SET last_processed_at = '2024-02-10T00:00:00Z', last_processed_tx_count = 2
            WHERE id = '{profile["id"]}';
            """
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "Default",
            "--interval", "month",
        )
        self._assert_ok(payload, result, "reports.balance-history")
        january = next(row for row in payload["data"] if row["period_start"] == "2024-01-01T00:00:00Z")
        february = next(row for row in payload["data"] if row["period_start"] == "2024-02-01T00:00:00Z")
        self.assertAlmostEqual(january["market_value"], 10000.0, places=4)
        self.assertAlmostEqual(february["cumulative_cost_basis"], 5000.0, places=4)
        self.assertAlmostEqual(february["market_value"], 10000.0, places=4)

    def test_table_output_honors_output_path(self):
        output_path = self.case_dir / "init.txt"
        result = self._run_cli("init", output=output_path)
        self.assertEqual(result.returncode, 0, msg=result.stderr)
        self.assertEqual(result.stdout.strip(), "")
        self.assertTrue(output_path.exists())
        text = output_path.read_text(encoding="utf-8")
        self.assertIn("version", text)
        self.assertIn("data_root", text)

    def test_migration_preserves_child_rows(self):
        self.data_root.mkdir(parents=True, exist_ok=True)
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.executescript(_OLD_SCHEMA_SQL)
        conn.executescript(
            """
            INSERT INTO workspaces VALUES('ws', 'Main', '2024-01-01T00:00:00Z');
            INSERT INTO profiles VALUES('pf', 'ws', 'Default', 'USD', 'generic', 365, 'FIFO', NULL, 0, '2024-01-01T00:00:00Z');
            INSERT INTO accounts VALUES('acct', 'ws', 'pf', 'cash', 'Cash', 'asset', 'BTC', '2024-01-01T00:00:00Z');
            INSERT INTO wallets VALUES('wal', 'ws', 'pf', 'acct', 'Wallet', 'address', '{}', '2024-01-01T00:00:00Z');
            INSERT INTO tags VALUES('tag', 'ws', 'pf', 'important', 'Important', '2024-01-01T00:00:00Z');
            INSERT INTO transactions VALUES('tx', 'ws', 'pf', 'wal', 'ext', 'fp', '2024-01-01T00:00:00Z', 'inbound', 'BTC', 1.0, 0.0, 'USD', 10000, 10000, 'deposit', 'desc', NULL, NULL, 0, '{}', '2024-01-01T00:00:00Z');
            INSERT INTO transaction_tags VALUES('tx', 'tag');
            INSERT INTO journal_entries VALUES('je', 'ws', 'pf', 'tx', 'wal', 'acct', '2024-01-01T00:00:00Z', 'acquisition', 'BTC', 1.0, 10000, 10000, NULL, NULL, NULL, 'desc', '2024-01-01T00:00:00Z');
            INSERT INTO journal_quarantines VALUES('tx', 'ws', 'pf', 'reason', '{}', '2024-01-01T00:00:00Z');
            """
        )
        conn.commit()
        conn.close()

        result = self._run_cli("status")
        self.assertEqual(result.returncode, 0, msg=result.stderr)

        conn = sqlite3.connect(db_path)
        counts = conn.execute(
            """
            SELECT
                (SELECT COUNT(*) FROM transactions),
                (SELECT COUNT(*) FROM transaction_tags),
                (SELECT COUNT(*) FROM journal_entries),
                (SELECT COUNT(*) FROM journal_quarantines)
            """
        ).fetchone()
        conn.close()
        self.assertEqual(counts, (1, 1, 1, 1))

    def test_invalid_import_timestamp_returns_validation_error(self):
        self._bootstrap_wallet(label="BadTS")
        json_file = self.case_dir / "bad-ts.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "not-a-timestamp",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BadTS",
            "--file", str(json_file),
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")


if __name__ == "__main__":
    unittest.main()
