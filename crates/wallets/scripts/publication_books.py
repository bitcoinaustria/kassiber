"""Synthetic current-schema books shared by the publication oracle generators.

Books contain only fixed synthetic identities; never real descriptors or keys.
Dumps carry every row, including trigger-maintained rows, so the Rust replay
loads tables before it creates triggers.
"""
from contextlib import ExitStack, contextmanager
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import sqlite3
from unittest.mock import patch

MANIFEST = Path(__file__).resolve().parents[2] / "store/fixtures/current-schema.json"


def new_book():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    manifest = json.loads(MANIFEST.read_text())
    for kind in ("table", "index", "view", "trigger"):
        for obj in manifest["objects"]:
            if obj[0] == kind and obj[3]:
                conn.execute(obj[3])
    conn.execute("INSERT INTO chain_index_clock VALUES(1,0)")
    conn.commit()
    return conn


def tables(conn):
    return [name for (name,) in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]


def dump(conn):
    """Every nonempty table's columns and rows, in rowid order."""
    result = {}
    for table in tables(conn):
        cursor = conn.execute(f'SELECT * FROM "{table}"')
        rows = [list(row) for row in cursor]
        if rows:
            result[table] = {"columns": [item[0] for item in cursor.description], "rows": rows}
    return result


def snapshot(conn):
    return {table: sorted([list(row) for row in conn.execute(f'SELECT * FROM "{table}"')],
                          key=lambda row: json.dumps(row, sort_keys=True))
            for table in tables(conn)}


def error(exc):
    return {"code": getattr(exc, "code", type(exc).__name__), "message": str(exc),
            "hint": getattr(exc, "hint", None), "details": getattr(exc, "details", None)}


def timestamp(step):
    """The `step`-th deterministic clock reading, one second apart."""
    return (datetime(2030, 1, 1, tzinfo=timezone.utc) + timedelta(seconds=step)).isoformat().replace("+00:00", "Z")


@contextmanager
def deterministic(*clock_modules, prefix="id"):
    """Replace uuid4 and the named modules' `now_iso` with fixed sequences."""
    counter = {"id": 0, "now": 0}

    def next_id():
        counter["id"] += 1
        return f"{prefix}-{counter['id']:04d}"

    def next_now():
        counter["now"] += 1
        return timestamp(counter["now"])

    with ExitStack() as stack:
        stack.enter_context(patch("uuid.uuid4", side_effect=next_id))
        for module in clock_modules:
            stack.enter_context(patch.object(module, "now_iso", side_effect=next_now))
        yield counter
