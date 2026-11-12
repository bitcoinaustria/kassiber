"""Reset book data keeps the user's decisions and binds them after the resync.

Owner decision D1 (2026-11-09): the reset rebuilds synced rows and journals,
keeps every decision, and re-attaches it once the resync brings its
transactions back. A decision whose transactions are missing or changed is
held, never applied elsewhere or dropped. "Wipe everything" stays available.

Every step runs through the product's CLI or daemon (`tests/cli_runner.py`),
so `KASSIBER_TEST_CLI` runs the same scenarios against the Rust binary.
"""

from __future__ import annotations

import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

from tests import cli_runner

WORKSPACE = "Demo"
PROFILE = "Main"
A_ROWS = [
    ("2023-01-02T10:00:00Z", "a-buy", "inbound", "1.00000000", "0", "20000"),
    ("2023-06-01T10:00:00Z", "a-out", "outbound", "0.50000000", "0.00001000", ""),
    ("2024-03-01T10:00:00Z", "a-loan", "outbound", "0.20000000", "0.00001000", ""),
    ("2024-05-01T10:00:00Z", "a-sell", "outbound", "0.10000000", "0", ""),
    ("2024-07-01T10:00:00Z", "a-gift", "inbound", "0.05000000", "0", ""),
]
B_ROWS = [
    ("2023-06-01T10:30:00Z", "b-in", "inbound", "0.50000000", "0", ""),
    ("2025-02-01T10:00:00Z", "b-sell", "outbound", "0.30000000", "0", ""),
]
RATES = (
    ("2023-01-02T10:00:00Z", "20000"),
    ("2023-06-01T10:00:00Z", "25000"),
    ("2023-06-01T10:30:00Z", "25000"),
    ("2024-03-01T10:00:00Z", "55000"),
    ("2024-05-01T10:00:00Z", "60000"),
    ("2024-07-01T10:00:00Z", "52000"),
    ("2025-02-01T10:00:00Z", "90000"),
)
DECISION_TABLES = (
    "transaction_pair_dismissals",
    "custody_components",
    "custody_component_legs",
    "custody_component_transaction_memberships",
    "loan_legs",
    "transaction_tags",
    "attachments",
    "source_funds_links",
    "transaction_edit_events",
    "transaction_edit_fields",
)


def _write_csv(path: Path, rows) -> None:
    lines = ["date,txid,direction,asset,amount,fee,fiat_rate"]
    lines += [",".join((date, txid, direction, "BTC", amount, fee, rate)) for date, txid, direction, amount, fee, rate in rows]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


class BookResetKeepsDecisionsTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="kassiber-book-reset-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.data_root = self.root / "data"
        # Children never resolve the owner's real app-data root.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env.setdefault(
            "KASSIBER_OFFLINE_PREFERENCE_FILE", str(self.root / "offline-mode.json")
        )
        self.env["KASSIBER_DISABLE_UPDATE_CHECK"] = "1"
        self.a_csv = self.root / "a.csv"
        self.b_csv = self.root / "b.csv"
        _write_csv(self.a_csv, A_ROWS)
        _write_csv(self.b_csv, B_ROWS)
        self.cli("init")
        self.cli("workspaces", "create", WORKSPACE)
        self.cli("profiles", "create", PROFILE, "--fiat-currency", "EUR")
        for label, path in (("A", self.a_csv), ("B", self.b_csv)):
            self.cli(
                "wallets", "create", "--label", label, "--kind", "custom",
                "--source-file", str(path), "--source-format", "csv",
            )
        for timestamp, rate in RATES:
            self.cli("rates", "set", "BTC-EUR", timestamp, rate)
        self.cli("wallets", "sync", "--all")
        self.cli("transfers", "pair", "--tx-out", "a-out", "--tx-in", "b-in", "--policy", "carrying-value")
        self.cli("loans", "mark", "--txid", "a-loan", "--as", "collateral", "--loan-id", "loan-1")
        self.cli("metadata", "notes", "set", "--transaction", "a-buy", "--note", "First purchase")
        self.cli("metadata", "tags", "create", "--code", "dca", "--label", "DCA")
        self.cli("metadata", "tags", "add", "--transaction", "a-buy", "--tag", "dca")
        self.cli("metadata", "records", "kind", "set", "--transaction", "a-gift", "--kind", "income")
        self.cli("attachments", "add", "--transaction", "a-buy", "--url", "https://example.invalid/receipt")
        self.cli(
            "source-funds", "links", "create", "--from-transaction", "a-buy",
            "--to-transaction", "a-out", "--type", "self_transfer", "--state", "reviewed",
        )
        self.cli("transfers", "dismiss", "--tx-out", "a-sell", "--tx-in", "a-gift", "--reason", "not a transfer")
        self.daemon("ui.transactions.metadata.update", {
            "transaction": self.tx_id("a-sell"), "fiat_rate": "61000",
            "pricing_source_kind": "manual_override", "reason": "Exchange statement",
        })
        self.cli("journals", "process")

    def cli(self, *args):
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "--machine", *args],
            capture_output=True, text=True, check=False, env=self.env,
        )
        try:
            payload = json.loads(result.stdout)
        except ValueError:
            raise AssertionError(f"{args} printed no JSON: {result.stdout[-2000:]} {result.stderr[-2000:]}")
        if result.returncode != 0 or payload.get("kind") == "error":
            raise AssertionError(f"{args} failed: {payload} {result.stderr[-2000:]}")
        return payload["data"]

    def daemon(self, kind, args):
        requests = [
            {"request_id": "call", "kind": kind, "args": args},
            {"request_id": "stop", "kind": "daemon.shutdown"},
        ]
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "daemon"],
            input="".join(json.dumps(request) + "\n" for request in requests),
            capture_output=True, text=True, check=False, env=self.env, timeout=600,
        )
        responses = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        answer = next(item for item in responses if item.get("request_id") == "call")
        self.assertNotEqual(answer.get("kind"), "error", answer)
        return answer["data"]

    def reset(self, **args):
        return self.daemon("ui.profiles.reset_data", {
            "confirm": "RESET", "confirm_profile": PROFILE,
            "auth_response": {"plaintext_delete_ack": "DELETE LOCAL DATA"}, **args,
        })

    def db(self):
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        self.addCleanup(conn.close)
        return conn

    def state(self):
        conn = self.db()
        entries = [
            {key: row[key] for key in row.keys() if key != "created_at"}
            for row in conn.execute("SELECT * FROM journal_entries ORDER BY id")
        ]
        decisions = {
            table: [tuple(row) for row in conn.execute(f"SELECT * FROM {table} ORDER BY rowid")]
            for table in DECISION_TABLES
        }
        metadata = [
            tuple(row)
            for row in conn.execute(
                "SELECT id, note, kind_override, fiat_rate, fiat_value_exact, pricing_source_kind "
                "FROM transactions ORDER BY id"
            )
        ]
        quarantines = [
            (row["transaction_id"], row["reason"], row["detail_json"])
            for row in conn.execute("SELECT * FROM journal_quarantines ORDER BY transaction_id, reason")
        ]
        gains = conn.execute(
            "SELECT printf('%.2f', coalesce(sum(gain_loss), 0)) FROM journal_entries WHERE gain_loss IS NOT NULL"
        ).fetchone()[0]
        return {"entries": entries, "decisions": decisions, "metadata": metadata,
                "quarantines": quarantines, "gains": gains}

    def tx_id(self, external_id):
        return self.db().execute(
            "SELECT id FROM transactions WHERE external_id = ?", (external_id,)
        ).fetchone()[0]

    def test_reset_then_sync_restores_identical_journals(self):
        before = self.state()
        self.assertEqual(before["quarantines"], [])
        self.assertEqual(len(before["decisions"]["loan_legs"]), 1)
        summary = self.reset()
        self.assertEqual(summary["mode"], "keep_decisions")
        restore = summary["decisions"]["bind_after_resync"]
        # The reviewed pair is a custody component: kept in place, its
        # memberships parked until both legs are back.
        self.assertEqual(summary["decisions"]["kept_in_place"]["custody_components"], 1)
        self.assertEqual(restore["custody_component_transaction_memberships"], 2)
        self.assertEqual(restore["loan_legs"], 1)
        self.assertEqual(restore["transaction_metadata"], 3)
        self.assertEqual(restore["transaction_pair_dismissals"], 1)
        self.assertEqual(summary["removed"]["transactions"], 7)
        self.assertNotIn("custody_components", summary["removed"])

        synced = self.cli("wallets", "sync", "--all")
        last = {item["wallet"]: item for item in synced}["B"]["reset_decisions"]
        self.assertEqual((last["held"], last["waiting"]), (0, 0))
        self.cli("journals", "process")
        after = self.state()
        self.assertEqual(after, before)
        conn = self.db()
        for table in ("book_reset_transactions", "book_reset_parked_rows"):
            self.assertEqual(conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0], 0, table)

    def test_missing_transaction_holds_its_decisions(self):
        out_id = self.tx_id("a-out")
        in_id = self.tx_id("b-in")
        self.reset()
        _write_csv(self.b_csv, [row for row in B_ROWS if row[1] != "b-in"])
        synced = self.cli("wallets", "sync", "--all")
        counts = {item["wallet"]: item for item in synced}["B"]["reset_decisions"]
        # The pair's two memberships bind together, so a-out holds both.
        self.assertEqual(counts["held"], 2)
        self.cli("journals", "process")
        conn = self.db()
        holds = conn.execute(
            "SELECT transaction_id, detail_json FROM journal_quarantines WHERE reason = 'book_reset_decision_held'"
        ).fetchall()
        self.assertEqual([row[0] for row in holds], [out_id])
        detail = json.loads(holds[0][1])
        self.assertEqual(detail["held_decisions"], {"custody_component_transaction_memberships": 2})
        self.assertEqual(
            detail["missing_transactions"],
            [{"transaction_id": in_id, "wallet_id": detail["missing_transactions"][0]["wallet_id"],
              "external_id": "b-in", "asset": "BTC", "direction": "inbound"}],
        )
        self.assertEqual(
            conn.execute("SELECT COUNT(*) FROM custody_component_transaction_memberships").fetchone()[0], 0
        )
        # The component itself still fails closed on its missing anchor.
        self.assertIn(
            "custody_quantity_unresolved",
            [item["reason"] for item in detail.get("additional_reasons", [])],
        )

    def test_changed_transaction_holds_its_decisions(self):
        loan_id = self.tx_id("a-loan")
        self.reset()
        _write_csv(
            self.a_csv,
            [(*row[:3], "0.25000000", *row[4:]) if row[1] == "a-loan" else row for row in A_ROWS],
        )
        self.cli("wallets", "sync", "--all")
        self.cli("journals", "process")
        conn = self.db()
        # The re-observed row keeps its id, but the mark is not applied to it.
        self.assertEqual(self.tx_id("a-loan"), loan_id)
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM loan_legs").fetchone()[0], 0)
        hold = conn.execute(
            "SELECT detail_json FROM journal_quarantines WHERE reason = 'book_reset_decision_held' AND transaction_id = ?",
            (loan_id,),
        ).fetchone()
        detail = json.loads(hold[0])
        self.assertEqual(detail["held_decisions"], {"loan_legs": 1})
        self.assertEqual(detail["changed_transactions"], [{"transaction_id": loan_id, "fields": ["amount"]}])
        self.assertEqual(detail["missing_transactions"], [])

    def test_wipe_everything_still_deletes_decisions(self):
        kept = self.reset()["decisions"]["bind_after_resync"]
        summary = self.reset(wipe_everything=True)
        self.assertEqual(summary["mode"], "wipe_everything")
        self.assertNotIn("decisions", summary)
        self.assertEqual(
            summary["removed"]["book_reset_parked_rows"],
            sum(count for table, count in kept.items() if table != "transaction_metadata"),
        )
        self.assertEqual(summary["removed"]["book_reset_transactions"], 7)
        self.cli("wallets", "sync", "--all")
        conn = self.db()
        for table in (*DECISION_TABLES, "book_reset_transactions", "book_reset_parked_rows"):
            self.assertEqual(conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0], 0, table)


class BookResetInventoryTest(unittest.TestCase):
    def test_every_row_a_transaction_delete_reaches_is_classified(self):
        """A new table under transactions must be parked or declared derived."""
        from kassiber.core import book_reset
        from kassiber.db import open_db

        with tempfile.TemporaryDirectory(prefix="kassiber-book-reset-schema-") as tmp:
            conn = open_db(Path(tmp) / "data")
            try:
                edges: dict[str, set[tuple[str, str]]] = {}
                for (table,) in conn.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
                ).fetchall():
                    for fk in conn.execute(f'PRAGMA foreign_key_list("{table}")').fetchall():
                        edges.setdefault(fk[2], set()).add((table, str(fk[6]).upper()))
            finally:
                conn.close()
        reached, frontier = set(), ["transactions"]
        while frontier:
            for child, on_delete in sorted(edges.get(frontier.pop(), ())):
                if child in reached or child == "transactions":
                    continue
                reached.add(child)
                if on_delete == "CASCADE":
                    frontier.append(child)
        classified = (
            {table for table, _, _ in book_reset.PARKED_TABLES}
            | book_reset.DERIVED_TRANSACTION_TABLES
            | book_reset.REANCHORED_TRANSACTION_TABLES
        )
        self.assertEqual(reached - classified, set())
        self.assertEqual(classified - reached, set())


if __name__ == "__main__":
    unittest.main()
