"""The actual SQLite driver must support safe WAL before Kassiber enables it."""
from unittest.mock import patch

import pytest

from kassiber import db
from kassiber.errors import AppError


@pytest.mark.parametrize("version,expected", [
    ("3.50.4", ("delete", "FULL")),
    ("3.51.1", ("delete", "FULL")),
    ("3.51.2", ("delete", "FULL")),
    ("3.51.3", ("wal", "NORMAL")),
    ("3.52.0", ("wal", "NORMAL")),
    ("3.44.5", ("delete", "FULL")),
    ("3.44.6", ("wal", "NORMAL")),
    ("3.44.7", ("wal", "NORMAL")),
    ("3.45.0", ("delete", "FULL")),
    ("3.50.6", ("delete", "FULL")),
    ("3.50.7", ("wal", "NORMAL")),
    ("3.50.8", ("wal", "NORMAL")),
    ("unknown", ("delete", "FULL")),
    ("3.51", ("delete", "FULL")),
    ("3.51.3-custom", ("delete", "FULL")),
    (None, ("delete", "FULL")),
])
def test_known_fixed_sqlite_branches_only(version, expected):
    assert db._journal_settings_for_sqlite_version(version) == expected


class VersionConnection:
    """Override only the version response; every PRAGMA reaches real SQLite."""
    def __init__(self, conn, version):
        self.conn = conn
        self.version = version

    def execute(self, sql):
        if sql == "SELECT sqlite_version()":
            return self.conn.execute("SELECT ?", (self.version,))
        return self.conn.execute(sql)


def test_persisted_wal_is_safely_converted_before_schema_writes(tmp_path):
    path = tmp_path / "journal.sqlite3"
    conn = db.sqlite3.connect(path)
    assert conn.execute("PRAGMA journal_mode=WAL").fetchone()[0] == "wal"
    conn.execute("CREATE TABLE marker(value)")
    conn.execute("INSERT INTO marker VALUES('preserved')")
    conn.commit()
    conn.close()
    conn = db.sqlite3.connect(path)
    try:
        db._configure_connection_pragmas(VersionConnection(conn, "3.50.4"))
        assert conn.execute("PRAGMA journal_mode").fetchone()[0] == "delete"
        assert conn.execute("PRAGMA synchronous").fetchone()[0] == 2
        assert conn.execute("SELECT value FROM marker").fetchone()[0] == "preserved"
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    finally:
        conn.close()


def test_fixed_driver_retains_wal_normal(tmp_path):
    conn = db.sqlite3.connect(tmp_path / "fixed.sqlite3")
    try:
        db._configure_connection_pragmas(VersionConnection(conn, "3.51.3"))
        assert conn.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
        assert conn.execute("PRAGMA synchronous").fetchone()[0] == 1
    finally:
        conn.close()


def test_active_wal_reader_prevents_unsafe_fallback_continuation(tmp_path):
    path = tmp_path / "held.sqlite3"
    writer = db.sqlite3.connect(path)
    writer.execute("PRAGMA journal_mode=WAL")
    writer.execute("CREATE TABLE marker(value)")
    writer.commit()
    reader = db.sqlite3.connect(path)
    try:
        reader.execute("BEGIN")
        reader.execute("SELECT * FROM marker").fetchall()
        with patch.object(db, "DB_BUSY_TIMEOUT_MS", 5), pytest.raises(AppError) as raised:
            db._configure_connection_pragmas(VersionConnection(writer, "3.50.4"))
        assert raised.value.code == "database_journal_mode_unavailable"
        assert raised.value.retryable is True
        assert writer.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
    finally:
        reader.close()
        writer.close()


def test_driver_cannot_silently_keep_unsafe_wal(tmp_path):
    conn = db.sqlite3.connect(tmp_path / "ignored.sqlite3")

    class IgnoredModeConnection(VersionConnection):
        def execute(self, sql):
            if sql == "PRAGMA journal_mode = delete":
                return self.conn.execute("SELECT 'wal'")
            return super().execute(sql)

    try:
        with pytest.raises(AppError) as raised:
            db._configure_connection_pragmas(IgnoredModeConnection(conn, "3.50.4"))
        assert raised.value.code == "database_journal_mode_unavailable"
        assert raised.value.details["actual_journal_mode"] == "wal"
    finally:
        conn.close()


def test_in_memory_accounting_clone_remains_supported():
    conn = db.sqlite3.connect(":memory:")
    try:
        db._configure_connection_pragmas(VersionConnection(conn, "unknown"))
        assert conn.execute("PRAGMA journal_mode").fetchone()[0] == "memory"
        assert conn.execute("PRAGMA synchronous").fetchone()[0] == 2
    finally:
        conn.close()
