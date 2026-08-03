"""Per-import rollback, end to end through the CLI.

The contract worth pinning: a rollback removes exactly the transactions one
import run *created*, leaves rows it only enriched alone, leaves the wallet
alone, and invalidates journals so reports are not silently stale afterwards.
"""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from kassiber.core import import_batches


REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

OPAQUE_CSV = (
    "Ausfuehrung,Richtung,Stueck,Spesen,Waehrung,Gegenwert\n"
    "2024-03-01T10:00:00Z,Kauf,0.5,0.0001,EUR,21000\n"
    "2024-04-02T10:00:00Z,Verkauf,0.25,0.0001,EUR,12000\n"
)
COLUMN_MAP = json.dumps(
    {
        "date": "Ausfuehrung",
        "type": "Richtung",
        "amount": "Stueck",
        "fee": "Spesen",
        "fiat_currency": "Waehrung",
        "fiat_value": "Gegenwert",
    }
)


class ImportRollbackCliTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.data_root = os.path.join(self._tmp.name, "data")
        self.csv_path = os.path.join(self._tmp.name, "xyz-export.csv")
        with open(self.csv_path, "w", encoding="utf-8") as handle:
            handle.write(OPAQUE_CSV)
        self.run_cli("init")
        self.run_cli("workspaces", "create", "W")
        self.run_cli(
            "profiles", "create", "P", "--workspace", "W",
            "--fiat-currency", "EUR", "--tax-country", "at",
        )
        self.run_cli(
            "wallets", "create", "--workspace", "W", "--profile", "P",
            "--label", "Exchange XYZ", "--kind", "custom",
        )
        self.addCleanup(self._tmp.cleanup)

    def run_cli(self, *args):
        completed = subprocess.run(
            [sys.executable, "-m", "kassiber", "--data-root", self.data_root,
             "--machine", *args],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            env={**os.environ, "PYTHONPYCACHEPREFIX": "/tmp/kassiber-pyc"},
        )
        payload = json.loads(completed.stdout) if completed.stdout.strip() else {}
        return payload

    def scoped(self, *args):
        return self.run_cli(*args, "--workspace", "W", "--profile", "P")

    def do_import(self):
        return self.scoped(
            "wallets", "import-ledger", "--wallet", "Exchange XYZ",
            "--file", self.csv_path, "--column-map", COLUMN_MAP,
        )["data"]

    def transaction_count(self):
        return len(self.scoped("transactions", "list")["data"])

    def batches(self):
        return self.scoped("imports", "list")["data"]["batches"]

    def test_an_import_is_recorded_as_a_rollback_able_run(self):
        outcome = self.do_import()
        self.assertEqual(outcome["imported"], 2)
        self.assertIn("import_batch_id", outcome)

        batches = self.batches()
        self.assertEqual(len(batches), 1)
        batch = batches[0]
        self.assertEqual(batch["source_format"], "generic_ledger")
        self.assertEqual(batch["source_filename"], "xyz-export.csv")
        self.assertEqual(batch["rows_inserted"], 2)
        self.assertEqual(batch["rows_present"], 2)
        self.assertFalse(batch["rolled_back"])
        # The confirmed plan is kept, so a recurring export can reuse it.
        self.assertEqual(batch["column_map"]["date"], "Ausfuehrung")

    def test_rollback_without_confirm_deletes_nothing(self):
        batch_id = self.do_import()["import_batch_id"]
        plan = self.scoped("imports", "rollback", "--batch", batch_id)["data"]
        self.assertFalse(plan["applied"])
        self.assertEqual(plan["transactions_to_delete"], 2)
        self.assertEqual(self.transaction_count(), 2)

    def test_rollback_removes_the_runs_transactions_but_keeps_the_wallet(self):
        batch_id = self.do_import()["import_batch_id"]
        self.scoped("journals", "process")

        result = self.scoped(
            "imports", "rollback", "--batch", batch_id, "--confirm"
        )["data"]
        self.assertTrue(result["rolled_back"])
        self.assertEqual(result["transactions_deleted"], 2)
        # Reports must not stay stale on pre-rollback numbers.
        self.assertTrue(result["journals_invalidated"])

        self.assertEqual(self.transaction_count(), 0)
        wallets = self.scoped("wallets", "list")["data"]
        self.assertEqual([w["label"] for w in wallets], ["Exchange XYZ"])
        self.assertEqual(self.batches(), [])

    def test_a_second_import_is_a_separate_run(self):
        # Importing the same file twice dedups, so use a distinct second file to
        # prove batches are independent rather than cumulative.
        first = self.do_import()["import_batch_id"]
        other = os.path.join(self._tmp.name, "second.csv")
        with open(other, "w", encoding="utf-8") as handle:
            handle.write(
                "Ausfuehrung,Richtung,Stueck,Spesen,Waehrung,Gegenwert\n"
                "2024-05-03T10:00:00Z,Kauf,0.75,0.0002,EUR,30000\n"
            )
        second = self.scoped(
            "wallets", "import-ledger", "--wallet", "Exchange XYZ",
            "--file", other, "--column-map", COLUMN_MAP,
        )["data"]["import_batch_id"]
        self.assertNotEqual(first, second)
        self.assertEqual(self.transaction_count(), 3)

        # Rolling back only the second run leaves the first run's rows.
        self.scoped("imports", "rollback", "--batch", second, "--confirm")
        self.assertEqual(self.transaction_count(), 2)
        remaining = [b["id"] for b in self.batches()]
        self.assertEqual(remaining, [first])

    def test_rolling_back_an_unknown_run_is_not_found(self):
        response = self.scoped("imports", "rollback", "--batch", "nope")
        self.assertEqual(response["kind"], "error")
        self.assertEqual(response["error"]["code"], "not_found")

    def test_an_import_that_only_enriched_rows_records_no_run(self):
        # A repeat of the same file inserts nothing, so there is nothing to roll
        # back and no misleading empty batch appears.
        self.do_import()
        repeat = self.do_import()
        self.assertEqual(repeat["imported"], 0)
        self.assertNotIn("import_batch_id", repeat)
        self.assertEqual(len(self.batches()), 1)


class RollbackScaleTests(unittest.TestCase):
    """A run's ids are bound parameters, and an exchange export is not small."""

    def build(self, row_count):
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.executescript(
            """
            PRAGMA foreign_keys = ON;
            CREATE TABLE workspaces(id TEXT PRIMARY KEY);
            CREATE TABLE profiles(id TEXT PRIMARY KEY);
            CREATE TABLE transactions(id TEXT PRIMARY KEY, profile_id TEXT);
            CREATE TABLE import_batches(
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
                wallet_id TEXT,
                source_format TEXT NOT NULL,
                source_filename TEXT,
                column_map_json TEXT,
                imported_at TEXT NOT NULL,
                rows_inserted INTEGER NOT NULL DEFAULT 0,
                rows_updated INTEGER NOT NULL DEFAULT 0,
                rows_skipped INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE import_batch_transactions(
                batch_id TEXT NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
                transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                PRIMARY KEY (batch_id, transaction_id)
            );
            CREATE TABLE transaction_tags(
                transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
                tag TEXT
            );
            CREATE TABLE transaction_pairs(
                id TEXT PRIMARY KEY,
                in_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,
                out_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE
            );
            CREATE TABLE pair_attachments(
                pair_id TEXT NOT NULL REFERENCES transaction_pairs(id) ON DELETE CASCADE,
                filename TEXT
            );
            INSERT INTO workspaces VALUES('w');
            INSERT INTO profiles VALUES('p');
            """
        )
        ids = [f"tx-{index}" for index in range(row_count)]
        conn.executemany("INSERT INTO transactions VALUES(?, 'p')", [(i,) for i in ids])
        conn.executemany("INSERT INTO transaction_tags VALUES(?, 'reviewed')", [(i,) for i in ids])
        # One pair row per two transactions, both sides created by this run.
        conn.executemany(
            "INSERT INTO transaction_pairs VALUES(?, ?, ?)",
            [(f"pair-{n}", ids[n], ids[n + 1]) for n in range(0, len(ids) - 1, 2)],
        )
        conn.executemany(
            "INSERT INTO pair_attachments VALUES(?, 'receipt.pdf')",
            [(f"pair-{n}",) for n in range(0, len(ids) - 1, 2)],
        )
        profile = {"id": "p", "workspace_id": "w"}
        batch_id = import_batches.record_batch(
            conn,
            profile,
            wallet_id=None,
            source_format="generic_ledger",
            source_filename="big-export.csv",
            column_map=None,
            outcome={"inserted_records": [{"transaction_id": i} for i in ids]},
        )
        return conn, profile, batch_id

    @unittest.skipUnless(
        hasattr(sqlite3.Connection, "setlimit"), "needs Python 3.11 setlimit"
    )
    def test_a_run_larger_than_sqlites_parameter_limit_still_rolls_back(self):
        # A years-long export exceeds SQLITE_MAX_VARIABLE_NUMBER, so binding one
        # parameter per id raises "too many SQL variables" and the user cannot
        # undo the very import they most want to undo. The ids are staged in a
        # temp table instead. The real limit is 32766; lowering it keeps the test
        # fast without changing what it proves.
        conn, profile, batch_id = self.build(1200)
        conn.setlimit(sqlite3.SQLITE_LIMIT_VARIABLE_NUMBER, 600)

        plan = import_batches.plan_rollback(conn, "p", batch_id)
        self.assertEqual(plan["transactions_to_delete"], 1200)
        self.assertEqual(plan["also_removed"].get("transaction_tags"), 1200)

        result = import_batches.rollback_batch(
            conn, profile, batch_id, invalidate_journals=lambda *_: None
        )
        self.assertEqual(result["transactions_deleted"], 1200)
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0], 0)

    def test_the_runs_own_link_rows_are_not_reported_as_collateral(self):
        # "Also removed: 1200 import_batch_transactions" is bookkeeping for the
        # thing being deleted, not user work the user should weigh.
        conn, _, batch_id = self.build(4)
        plan = import_batches.plan_rollback(conn, "p", batch_id)
        self.assertNotIn("import_batch_transactions", plan["also_removed"])

    def test_collateral_counts_rows_once_and_follows_transitive_cascades(self):
        # Two failures in one count. A pair row names two transactions and a run
        # usually creates both sides, so counting per foreign key reported it
        # twice; and a row that dies with the *pair* rather than with the
        # transaction was not counted at all, under-reporting authored evidence
        # the rollback destroys — which is the one thing the plan exists to say.
        conn, _, batch_id = self.build(4)
        plan = import_batches.plan_rollback(conn, "p", batch_id)
        self.assertEqual(plan["also_removed"]["transaction_tags"], 4)
        self.assertEqual(plan["also_removed"]["transaction_pairs"], 2)
        self.assertEqual(plan["also_removed"]["pair_attachments"], 2)


if __name__ == "__main__":
    unittest.main()
