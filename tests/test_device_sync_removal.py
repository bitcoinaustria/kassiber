"""Device sync was removed (plan 19); older books lose only its own objects."""

from __future__ import annotations

import base64
import gzip
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.db import (
    DEVICE_SYNC_REMOVAL_MIGRATION,
    RETIRED_DEVICE_SYNC_COLUMNS,
    RETIRED_DEVICE_SYNC_INDEXES,
    RETIRED_DEVICE_SYNC_TABLES,
    _drop_device_sync_schema,
    open_db,
)


FIXTURE = Path(__file__).parent / "fixtures" / "historical" / "pre_435_16b7bdc1.sqlite3.gz.b64"


def _sync_objects(conn) -> set[str]:
    names = (*RETIRED_DEVICE_SYNC_TABLES, *RETIRED_DEVICE_SYNC_INDEXES)
    placeholders = ", ".join("?" for _ in names)
    return {
        str(row[0])
        for row in conn.execute(
            f"SELECT name FROM sqlite_master WHERE name IN ({placeholders})", names
        )
    }


def _core_row_counts(conn) -> dict[str, int]:
    tables = [
        str(row[0])
        for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' "
            "AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'sync\\_%' ESCAPE '\\'"
        )
    ]
    return {
        table: int(conn.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0])
        for table in tables
    }


class DeviceSyncRemovalTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="kassiber-sync-removal-")
        self.addCleanup(temporary.cleanup)
        self.data_root = Path(temporary.name) / "data"
        self.data_root.mkdir(parents=True)

    def _install_fixture(self) -> Path:
        database = self.data_root / "kassiber.sqlite3"
        database.write_bytes(gzip.decompress(base64.b64decode(FIXTURE.read_bytes())))
        return database

    def test_new_book_has_no_device_sync_objects(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        self.assertEqual(set(), _sync_objects(conn))
        edit_columns = {
            str(row["name"])
            for row in conn.execute("PRAGMA table_info(transaction_edit_events)")
        }
        self.assertTrue(
            edit_columns.isdisjoint(RETIRED_DEVICE_SYNC_COLUMNS["transaction_edit_events"])
        )
        self.assertIsNone(
            conn.execute(
                "SELECT 1 FROM schema_migration_audits WHERE migration_name = ?",
                (DEVICE_SYNC_REMOVAL_MIGRATION,),
            ).fetchone()
        )

    def test_replicating_book_drops_sync_objects_and_keeps_authored_rows(self):
        database = self._install_fixture()
        legacy = sqlite3.connect(database)
        try:
            self.assertTrue(_sync_objects(legacy) >= {"sync_events", "sync_conflicts"})
            self.assertEqual(2, legacy.execute("SELECT COUNT(*) FROM sync_events").fetchone()[0])
            before = _core_row_counts(legacy)
        finally:
            legacy.close()

        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        self.assertEqual(set(), _sync_objects(conn))
        self.assertEqual("ok", conn.execute("PRAGMA integrity_check").fetchone()[0])
        after = _core_row_counts(conn)
        for table, count in before.items():
            if table in after:
                # Migrations may add derived rows; authored rows never disappear.
                self.assertGreaterEqual(after[table], count, table)
        for table in ("transactions", "wallets", "transaction_pairs", "transaction_edit_events"):
            self.assertEqual(before[table], after[table], table)
        audit = conn.execute(
            "SELECT impact_json FROM schema_migration_audits WHERE migration_name = ?",
            (DEVICE_SYNC_REMOVAL_MIGRATION,),
        ).fetchone()
        change = json.loads(audit["impact_json"])["changes"][0]
        self.assertEqual(2, change["signed_event_count"])
        self.assertEqual(1, change["open_conflict_count"])

    def test_removal_step_is_read_only_once_applied(self):
        # Other open-time migrations own their own idempotence; this step must
        # only probe sqlite_master on an already-migrated or fresh book.
        self._install_fixture()
        for _ in range(2):
            conn = open_db(self.data_root)
            self.addCleanup(conn.close)
            conn.commit()
            statements: list[str] = []
            conn.set_trace_callback(statements.append)
            changes = conn.total_changes
            self.assertEqual(0, _drop_device_sync_schema(conn))
            conn.set_trace_callback(None)
            self.assertEqual(changes, conn.total_changes)
            self.assertFalse(conn.in_transaction)
            self.assertEqual(1, len(statements))
            self.assertTrue(statements[0].lstrip().upper().startswith("SELECT"))


if __name__ == "__main__":
    unittest.main()
