"""A SQLite database as comparable data: schema objects and every row.

The dump opens a scratch copy, never the recorded file, so reading cannot
checkpoint or migrate the evidence. A write-ahead log captured next to the
database is copied along and applied by SQLite on open.
"""

from __future__ import annotations

import shutil
import sqlite3
import tempfile
from dataclasses import dataclass, field
from pathlib import Path


SQLITE_HEADER = b"SQLite format 3\x00"
SIDE_SUFFIXES = ("-wal", "-journal")
DATABASE_SUFFIXES = (".sqlite3", ".sqlite", ".db")
# SQLCipher's parameters are part of the storage contract (plan 21).
SQLCIPHER_PRAGMAS = ("PRAGMA cipher_compatibility = 4", "PRAGMA kdf_iter = 256000", "PRAGMA cipher_page_size = 4096")


def is_sqlite(data: bytes) -> bool:
    return data.startswith(SQLITE_HEADER)


@dataclass
class Table:
    name: str
    columns: list[str]
    pk: list[str]
    has_rowid: bool
    # (rowid or None, values) in primary-key order (rowid order without one).
    rows: list[tuple[int | None, tuple]] = field(default_factory=list)


@dataclass
class Dump:
    pragmas: dict[str, object]
    schema: list[tuple[str, str, str, str | None]]
    tables: dict[str, Table]


class UnreadableDatabase(Exception):
    pass


def dump_files(main: bytes, side: dict[str, bytes] | None = None, *, keys=()) -> Dump:
    """Dump a database given its file contents (and side files by suffix).

    An encrypted database opens with the first of `keys` (synthetic
    passphrases the session fed in) that reads it.
    """
    with tempfile.TemporaryDirectory(prefix="kassiber-session-dump-") as scratch:
        path = Path(scratch) / "db.sqlite3"
        path.write_bytes(main)
        for suffix, data in (side or {}).items():
            Path(f"{path}{suffix}").write_bytes(data)
        return dump_path(path, keys=keys)


def dump_copy(source: Path) -> Dump:
    with tempfile.TemporaryDirectory(prefix="kassiber-session-dump-") as scratch:
        path = Path(scratch) / "db.sqlite3"
        shutil.copyfile(source, path)
        for suffix in SIDE_SUFFIXES:
            side = Path(f"{source}{suffix}")
            if side.is_file():
                shutil.copyfile(side, f"{path}{suffix}")
        return dump_path(path)


def _open(path: Path, keys):
    if path.read_bytes()[: len(SQLITE_HEADER)] == SQLITE_HEADER or not keys:
        return sqlite3.connect(path)
    import sqlcipher3

    # The latest passphrase most likely keys the final state; wrong ones are
    # expected, so the library's own error log stays quiet.
    for key in reversed(list(keys)):
        conn = sqlcipher3.connect(str(path))
        conn.execute("PRAGMA cipher_log_level = NONE")
        conn.execute("PRAGMA key = '" + key.replace("'", "''") + "'")
        for pragma in SQLCIPHER_PRAGMAS:
            conn.execute(pragma)
        try:
            conn.execute("SELECT count(*) FROM sqlite_master").fetchone()
        except sqlcipher3.DatabaseError:
            conn.close()
            continue
        return conn
    raise UnreadableDatabase("encrypted, and no recorded passphrase opens it")


def dump_path(path: Path, *, keys=()) -> Dump:
    try:
        conn = _open(Path(path), keys)
    except sqlite3.Error as exc:
        raise UnreadableDatabase(str(exc)) from exc
    try:
        conn.text_factory = _text
        try:
            schema = [
                tuple(row)
                for row in conn.execute(
                    "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name"
                )
            ]
        except Exception as exc:  # sqlite3 and sqlcipher3 raise unrelated classes
            raise UnreadableDatabase(str(exc)) from exc
        pragmas = {
            name: conn.execute(f"PRAGMA {name}").fetchone()[0]
            for name in ("user_version", "application_id", "encoding", "page_size", "auto_vacuum", "journal_mode")
        }
        tables = {}
        for kind, name, _tbl, sql in schema:
            if kind != "table" or (sql or "").upper().startswith("CREATE VIRTUAL TABLE"):
                continue
            tables[name] = _dump_table(conn, name, sql or "")
        return Dump(pragmas=pragmas, schema=schema, tables=tables)
    finally:
        conn.close()


def _text(data: bytes):
    # Stored text that is not UTF-8 stays bytes, so it compares exactly.
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return data


def _quote(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def _dump_table(conn: sqlite3.Connection, name: str, sql: str) -> Table:
    info = conn.execute(f"PRAGMA table_info({_quote(name)})").fetchall()
    columns = [row[1] for row in info]
    pk = [row[1] for row in sorted((row for row in info if row[5]), key=lambda row: row[5])]
    has_rowid = "WITHOUT ROWID" not in sql.upper()
    selected = ", ".join(_quote(column) for column in columns)
    order = ", ".join(_quote(column) for column in pk) if pk else "rowid"
    if has_rowid:
        query = f"SELECT rowid, {selected} FROM {_quote(name)} ORDER BY {order}, rowid"
        rows = [(row[0], tuple(row[1:])) for row in conn.execute(query)]
    else:
        query = f"SELECT {selected} FROM {_quote(name)} ORDER BY {order}"
        rows = [(None, tuple(row)) for row in conn.execute(query)]
    return Table(name=name, columns=columns, pk=pk, has_rowid=has_rowid, rows=rows)
