"""Books from earlier versions may still contain removed general-ledger storage."""

import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber.core import legacy_ledger, maintenance
from kassiber.db import open_db
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parent.parent

# A representative subset of the removed schema: book-owned rows with RESTRICT
# foreign keys, a parent/child pair, retention triggers, and the three triggers
# the ledger installed on the core external_documents table.
LEGACY_DDL = """
CREATE TABLE gl_books (
    profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE RESTRICT,
    currency TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE gl_entries (
    id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
    description TEXT NOT NULL, UNIQUE(profile_id, id)
);
CREATE TABLE gl_lines (
    entry_id TEXT NOT NULL, profile_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
    PRIMARY KEY(entry_id, ordinal),
    FOREIGN KEY(profile_id, entry_id) REFERENCES gl_entries(profile_id, id) ON DELETE RESTRICT
);
CREATE TABLE gl_evidence (
    id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
    content BLOB NOT NULL,
    source_document_id TEXT REFERENCES external_documents(id) ON DELETE RESTRICT
);
CREATE TRIGGER gl_evidence_no_delete BEFORE DELETE ON gl_evidence BEGIN
    SELECT RAISE(ABORT,'accounting_evidence_retained'); END;
CREATE TRIGGER gl_lines_no_delete BEFORE DELETE ON gl_lines BEGIN
    SELECT RAISE(ABORT,'accounting_entry_immutable'); END;
CREATE TRIGGER gl_evidence_document_retained
    BEFORE DELETE ON external_documents WHEN EXISTS
    (SELECT 1 FROM gl_evidence WHERE source_document_id=OLD.id)
    BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END;
CREATE TRIGGER gl_evidence_document_scope_retained
    BEFORE UPDATE ON external_documents WHEN (NEW.id!=OLD.id OR NEW.profile_id!=OLD.profile_id)
    AND EXISTS (SELECT 1 FROM gl_evidence WHERE source_document_id=OLD.id)
    BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END;
CREATE TRIGGER gl_evidence_document_no_replace
    BEFORE INSERT ON external_documents WHEN EXISTS
    (SELECT 1 FROM gl_evidence WHERE source_document_id=NEW.id)
    BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END;
"""


def _run_cli(data_root, *args):
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--data-root", str(data_root), "--machine", *args],
        cwd=ROOT, capture_output=True, text=True, check=False,
    )
    payload = json.loads(result.stdout.strip())
    if result.returncode != 0 or payload.get("kind") == "error":
        raise AssertionError(f"CLI failed for {args}: {payload}; stderr={result.stderr}")
    return payload


def _legacy_objects(conn):
    return sorted(
        tuple(row) for row in conn.execute(
            r"SELECT type, name FROM sqlite_master WHERE name LIKE 'gl\_%' ESCAPE '\'"
        )
    )


class LegacyLedgerTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-legacy-ledger-")
        self.addCleanup(self._tmp.cleanup)
        self.data_root = Path(self._tmp.name) / "data"
        _run_cli(self.data_root, "init")
        _run_cli(self.data_root, "workspaces", "create", "Demo")
        _run_cli(self.data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
        conn = open_db(str(self.data_root))
        try:
            row = conn.execute("SELECT id, workspace_id FROM profiles WHERE label = 'Main'").fetchone()
            self.profile_id, self.workspace_id = row["id"], row["workspace_id"]
        finally:
            conn.close()

    def _open(self):
        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        return conn

    def _install_legacy(self, *, populated):
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.execute("PRAGMA foreign_keys = ON")
        conn.executescript(LEGACY_DDL)
        conn.execute(
            "INSERT INTO external_documents(id, workspace_id, profile_id, document_type, label, created_at, updated_at) "
            "VALUES ('doc-1', ?, ?, 'invoice', 'Invoice', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            (self.workspace_id, self.profile_id),
        )
        if populated:
            conn.execute("INSERT INTO gl_books VALUES (?, 'EUR', '2026-01-01T00:00:00Z')", (self.profile_id,))
            conn.execute("INSERT INTO gl_entries VALUES ('entry-1', ?, 'Opening')", (self.profile_id,))
            conn.execute("INSERT INTO gl_lines VALUES ('entry-1', ?, 1)", (self.profile_id,))
            conn.execute("INSERT INTO gl_evidence VALUES ('ev-1', ?, x'00', 'doc-1')", (self.profile_id,))
        conn.commit()
        conn.close()

    def test_new_database_has_no_ledger_objects(self):
        self.assertEqual(_legacy_objects(self._open()), [])

    def test_empty_legacy_tables_and_document_triggers_are_dropped_on_open(self):
        self._install_legacy(populated=False)
        conn = self._open()
        self.assertEqual(_legacy_objects(conn), [])
        conn.execute("DELETE FROM external_documents WHERE id = 'doc-1'")
        conn.commit()

    def test_populated_legacy_tables_survive_open_and_keep_document_guards(self):
        self._install_legacy(populated=True)
        conn = self._open()
        self.assertIn(("trigger", "gl_evidence_document_retained"), _legacy_objects(conn))
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM gl_lines").fetchone()[0], 1)
        with self.assertRaisesRegex(sqlite3.IntegrityError, "accounting_evidence_retained"):
            conn.execute("DELETE FROM external_documents WHERE id = 'doc-1'")
        with self.assertRaisesRegex(sqlite3.IntegrityError, "accounting_evidence_retained"):
            conn.execute("UPDATE external_documents SET id = 'doc-2' WHERE id = 'doc-1'")
        conn.rollback()
        # Metadata edits that do not move the document remain allowed.
        conn.execute("UPDATE external_documents SET label = 'Renamed' WHERE id = 'doc-1'")
        conn.commit()

    def test_populated_legacy_rows_block_core_delete_paths(self):
        self._install_legacy(populated=True)
        conn = self._open()
        with self.assertRaises(AppError) as raised:
            maintenance.reset_current_profile_data(conn, str(self.data_root))
        self.assertEqual(raised.exception.code, "legacy_ledger_present")
        self.assertIn("purge-legacy-ledger", raised.exception.hint)
        with self.assertRaises(AppError):
            legacy_ledger.require_absent(conn, profile_ids=[self.profile_id], action="deleting")
        legacy_ledger.require_absent(conn, profile_ids=["other-book"], action="deleting")
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0], 1)

    def test_repeat_open_probe_writes_nothing(self):
        for populated in (True, False):
            with self.subTest(populated=populated):
                if populated:
                    self._install_legacy(populated=True)
                conn = self._open()
                writes = []

                def authorizer(action, *_):
                    if action not in (sqlite3.SQLITE_SELECT, sqlite3.SQLITE_READ, sqlite3.SQLITE_PRAGMA,
                                      sqlite3.SQLITE_FUNCTION):
                        writes.append(action)
                    return sqlite3.SQLITE_OK

                conn.set_authorizer(authorizer)
                before = conn.total_changes
                self.assertFalse(legacy_ledger.retire_if_empty(conn))
                conn.set_authorizer(None)
                self.assertEqual(writes, [])
                self.assertEqual(conn.total_changes, before)
                self.assertFalse(conn.in_transaction)
                if populated:
                    legacy_ledger.purge(conn)

    def test_purge_requires_confirmation_then_removes_all_ledger_objects(self):
        self._install_legacy(populated=True)
        plan = _run_cli(self.data_root, "maintenance", "purge-legacy-ledger")
        self.assertEqual(plan["kind"], "maintenance.purge-legacy-ledger.plan")
        self.assertFalse(plan["data"]["applied"])
        self.assertEqual(plan["data"]["row_count"], 4)
        self.assertIn("backup", plan["data"]["hint"])
        conn = self._open()
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM gl_evidence").fetchone()[0], 1)
        conn.close()

        purged = _run_cli(self.data_root, "maintenance", "purge-legacy-ledger", "--confirm")
        self.assertTrue(purged["data"]["applied"])
        conn = self._open()
        self.assertEqual(_legacy_objects(conn), [])
        conn.execute("DELETE FROM external_documents WHERE id = 'doc-1'")
        conn.commit()
        self.assertEqual(maintenance.reset_current_profile_data(conn, str(self.data_root))["reset"], True)


if __name__ == "__main__":
    unittest.main()
