"""Freeze the reference's empty-book writes without changing Python's DDL.

This is a development tool, never a runtime dependency. The transcript is only
valid on an empty schema. Existing books must use individually ported migrations.
Run with ``uv run --locked python crates/store/tools/export_creation.py --check``.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sqlite3
import tempfile
from unittest.mock import patch

from kassiber import db

DESTINATION = Path(__file__).resolve().parents[1] / "fixtures/creation.json"


def creation_statements():
    statements = []
    original = sqlite3.connect

    def traced(*args, **kwargs):
        conn = original(*args, **kwargs)
        conn.set_trace_callback(statements.append)
        return conn

    with tempfile.TemporaryDirectory(prefix="kassiber-r3-schema-") as directory:
        with patch.object(db.sqlite3, "connect", traced):
            conn = db.open_db(Path(directory) / "data")
        conn.close()
    writes = []
    for statement in statements:
        sql = re.sub(r"(?m)^\s*--[^\n]*(?:\n|$)", "", statement).strip()
        if re.match(r"^(CREATE|ALTER|DROP|INSERT|UPDATE|DELETE)\b", sql, re.I):
            if "INSERT INTO settings(key, value)" in sql and "database_instance_id" in sql:
                continue
            if "'custody-legacy-pair-fee-review-v1'" in sql:
                # The audit binds Python's now_iso(); replay must stamp its own
                # creation time in that format, not this export's.
                statement, count = re.subn(
                    r"'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ'\)",
                    "strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))", statement,
                )
                assert count == 1, "legacy pair fee audit timestamp moved"
            writes.append(statement)
    return {"format_version": 1, "statements": writes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(creation_statements(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        assert DESTINATION.read_text() == encoded, "Python empty-book writes changed; review Rust creation"
    else:
        DESTINATION.write_text(encoded)


if __name__ == "__main__":
    main()
