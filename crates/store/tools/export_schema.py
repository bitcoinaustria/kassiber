"""Freeze Python open_db's post-compat schema on a disposable data root.

Run from the repository root with uv run --locked python
crates/store/tools/export_schema.py [--check]. This is a test/development tool,
not a runtime Python dependency. Review every manifest change as a schema gate.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import tempfile

from kassiber.db import open_db

DESTINATION = Path(__file__).resolve().parents[1] / "fixtures/current-schema.json"


def snapshot(conn):
    objects = [list(row) for row in conn.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master "
        "WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name"
    )]
    audits = [list(row) for row in conn.execute(
        "SELECT id, migration_name, schema_version FROM schema_migration_audits "
        "ORDER BY id"
    )]
    return {"schema_version": 1, "reference": "ce382b13", "objects": objects, "required_audits": audits}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="kassiber-r1-schema-") as root:
        conn = open_db(Path(root) / "data")
        try:
            payload = snapshot(conn)
        finally:
            conn.close()
    encoded = json.dumps(payload, indent=2, ensure_ascii=True) + "\n"
    if args.check:
        assert DESTINATION.read_text() == encoded, "Python schema changed; review the Rust schema gate"
    else:
        DESTINATION.write_text(encoded)


if __name__ == "__main__":
    main()
