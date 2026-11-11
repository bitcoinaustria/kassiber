"""Freeze Python's migration of legacy books as Rust store vectors.

The Rust store's migrating open runs the custody steps Python's
`ensure_schema_compat` hands to the custody core (legacy reviewed
pair/payout consolidation, the legacy pair fee review, componentless
dismissal recovery). Each vector is an input book and the complete typed dump
of the book after Python's `open_db` migrated it with a pinned clock and
database identity: every schema object, every table, every row, each value
with its SQLite storage class (REAL as its IEEE-754 bits).
`crates/transfers/tests/legacy_migrations.rs` migrates the same input with the
Rust binary's open and requires the identical dump.

    python -m tests.rust_port.gen_legacy_migration_vectors --out DIR INPUT...

INPUT is a plaintext `.sqlite3` book, or a `.sqlite3.gz.b64` historical
fixture. The vector name is the input's file name without its suffixes.
Writes `DIR/<name>.input.sqlite3.gz` and `DIR/<name>.expected.json.gz`.
"""
from __future__ import annotations

import argparse
import base64
import gzip
import json
import os
from pathlib import Path
import sqlite3
import struct
import tempfile
from unittest.mock import patch
import uuid

from kassiber import db
from kassiber.core import custody_authored_migration, custody_components
from kassiber.operator import project as ownership

IDENTITY = "0123456789ab4def8123456789abcdef"
# The store's `strftime` override returns this verbatim; Python's `now_iso()`
# is pinned to it too. The component layer's `datetime.now().isoformat()`
# writes the same instant with microseconds, so a step that read the wrong
# clock shows up in the dump.
TIMESTAMP = "2026-10-27T12:34:56.789Z"
PRECISE = "2026-10-27T12:34:56.789000Z"


def quote(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def typed(kind: str, value):
    if kind == "real":
        return [kind, struct.pack(">d", value).hex()]
    if kind == "blob":
        return [kind, bytes(value).hex()]
    return [kind, value]


def dump(conn: sqlite3.Connection) -> dict:
    """Every schema object and every row, typed, in primary-key order."""
    objects = [
        list(row)
        for row in conn.execute(
            "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name"
        )
    ]
    tables = {}
    for (table,) in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    ).fetchall():
        columns = list(conn.execute(f"PRAGMA table_info({quote(table)})"))
        primary = [row[1] for row in sorted(columns, key=lambda row: row[5]) if row[5]]
        order = list(dict.fromkeys(primary + [row[1] for row in columns]))
        kinds = ",".join(f"typeof({quote(row[1])})" for row in columns)
        rows = conn.execute(
            f"SELECT *,{kinds} FROM {quote(table)} ORDER BY {','.join(map(quote, order))}"
        ).fetchall()
        width = len(columns)
        tables[table] = {
            "columns": [row[1] for row in columns],
            "rows": [
                [typed(kind, value) for kind, value in zip(row[width:], row[:width])]
                for row in rows
            ],
        }
    return {"objects": objects, "tables": tables}


def python_migrate(root: Path) -> dict:
    original = sqlite3.connect

    def deterministic(*args, **kwargs):
        conn = original(*args, **kwargs)
        conn.create_function("strftime", 2, lambda *_: TIMESTAMP)
        return conn

    with patch.object(db.sqlite3, "connect", deterministic), \
            patch.object(db.uuid, "uuid4", return_value=uuid.UUID(hex=IDENTITY)), \
            patch.object(custody_authored_migration, "now_iso", return_value=TIMESTAMP), \
            patch.object(custody_components, "_now_iso", return_value=PRECISE):
        conn = db.open_db(root)
    try:
        conn.row_factory = None
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
        return dump(conn)
    finally:
        conn.close()


def read_input(path: Path) -> bytes:
    raw = path.read_bytes()
    if path.name.endswith(".gz.b64"):
        return gzip.decompress(base64.b64decode(raw))
    return raw


def vector_name(path: Path) -> str:
    name = path.name
    for suffix in (".b64", ".gz", ".sqlite3"):
        name = name.removesuffix(suffix)
    return name


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("inputs", nargs="+", help="PATH or NAME=PATH")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    for spec in args.inputs:
        name, _, given = spec.rpartition("=")
        path = Path(given)
        name = name or vector_name(path)
        book = read_input(path)
        with tempfile.TemporaryDirectory(prefix="legacy-vector-") as scratch:
            scratch = Path(scratch)
            root = scratch / "data"
            root.mkdir()
            (root / "kassiber.sqlite3").write_bytes(book)
            # Disposable account state: no real data root, owner lock or
            # network reach.
            home = scratch / "home"
            locks = scratch / "owners"
            for directory in (home, locks):
                directory.mkdir(mode=0o700)
            environment = {
                "HOME": str(home),
                "XDG_DATA_HOME": str(home / "xdg"),
                "KASSIBER_NO_EGRESS": "1",
            }
            with patch.dict(os.environ, environment), \
                    patch.object(ownership, "_owner_lock_root", return_value=locks):
                expected = python_migrate(root)
        # Fixed gzip headers keep regenerated vectors byte-stable.
        (args.out / f"{name}.input.sqlite3.gz").write_bytes(gzip.compress(book, mtime=0))
        encoded = json.dumps(expected, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        (args.out / f"{name}.expected.json.gz").write_bytes(
            gzip.compress(encoded.encode("utf-8"), mtime=0)
        )
        print(f"{name}: {sum(len(t['rows']) for t in expected['tables'].values())} rows")


if __name__ == "__main__":
    main()
