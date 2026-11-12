#!/usr/bin/env python3
"""Freeze the shared report CSV writer at every `_export_csv_rows` export.

Round 5 (133cde709, 1750ae701, 5a7cf056a, 42f1e342f, 20102d44a, 1168e6e23,
6d14a1825) made full-report, transaction-list and Austrian bundle CSVs
create their own parents, write devices, pipes and directories in place,
stage ordinary files under short private names, type storage failures as
`export_write_failed` and skip saved-snapshot registration for non-regular
targets. Each case records the result or error, the tables the export
changed and every name left in the artifact root (no staged leftovers).
"""
from pathlib import Path
from dataclasses import replace
import json
import os
import sys
import tempfile
import threading
from uuid import UUID
from unittest.mock import patch

import kassiber.cli.main as cli
from kassiber.core import reports, custody_filed_reports as snapshots
from kassiber.db import open_db
from kassiber.errors import AppError
from gen_artifact_vectors import FixedDateTime
from gen_export_sequences import CHAIN, STAMP, dump

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "crates/reports/tests/data/csv_write.json"
SEED = ("DROP TRIGGER trg_schema_migration_audits_immutable;"
        "UPDATE schema_migration_audits SET created_at='seeded';"
        "CREATE TRIGGER trg_schema_migration_audits_immutable BEFORE UPDATE ON schema_migration_audits "
        "BEGIN SELECT RAISE(ABORT, 'schema_migration_audits_immutable'); END;")
LONG = "r" * 240 + ".csv"
COMMANDS = {
    "reports.export-csv": ("generic", {"history_limit": 0}),
    "reports.export-transactions-csv": ("generic", {}),
    "reports.export-austrian-e1kv-csv": ("at", {"year": 2024}),
}
# Target name -> (setup, target relative to the root or absolute).
TARGETS = {
    "new-nested": "a/b/report.csv",
    "existing": "existing.csv",
    "final-symlink": "link.csv",
    "parent-file": "parent/missing/report.csv",
    # Python formats the requested path as a pathlib.Path: doubled
    # separators, "." parts and a trailing separator do not survive.
    "unnormalized-parent-file": "parent//missing/./report.csv",
    "trailing-separator-directory": "directory.csv/",
    "read-only-parent": "locked/missing/report.csv",
    "directory": "directory.csv",
    "long-name": LONG,
    "dev-null": "/dev/null",
    "fifo": "report.pipe",
}


def prepare(root: Path, name: str) -> None:
    if name == "existing":
        (root / "existing.csv").write_bytes(b"old\r\n")
    elif name == "final-symlink":
        (root / "real.csv").write_bytes(b"old\r\n")
        (root / "link.csv").symlink_to("real.csv")
    elif name in ("parent-file", "unnormalized-parent-file"):
        (root / "parent").write_text("file")
    elif name == "read-only-parent":
        (root / "locked").mkdir()
        (root / "locked").chmod(0o500)
    elif name in ("directory", "trailing-separator-directory"):
        (root / "directory.csv").mkdir()
    elif name == "fifo":
        os.mkfifo(root / "report.pipe")


def run(conn, command, args, target):
    with patch.object(reports, "datetime", FixedDateTime), patch.object(snapshots, "now_iso", return_value=STAMP), patch(
        "kassiber.core.filed_report_chain.now_iso", return_value=STAMP
    ), patch.object(snapshots.uuid, "uuid4", return_value=UUID(int=1)):
        if command == "reports.export-csv":
            return reports.export_csv_report(conn, None, None, target, replace(cli._report_hooks(), now_iso=lambda: STAMP),
                                             history_limit=args["history_limit"])
        if command == "reports.export-transactions-csv":
            return reports.export_transactions_csv_report(conn, None, None, target, cli._report_hooks())
        return reports.export_austrian_e1kv_csv_bundle(conn, None, None, target, cli._report_hooks(), tax_year=args["year"])


def build():
    if os.geteuid() == 0:
        raise SystemExit("run as an ordinary user: root ignores the read-only parent")
    cases = []
    for command, (country, args) in COMMANDS.items():
        for name, relative in TARGETS.items():
            bundle = command == "reports.export-austrian-e1kv-csv"
            if bundle and name in ("existing", "final-symlink", "long-name", "fifo"):
                continue
            sql = SEED + f"UPDATE profiles SET tax_country='{country}' WHERE id='p';" + CHAIN
            with tempfile.TemporaryDirectory(prefix="csv-write-") as root_text:
                root = Path(root_text)
                conn = open_db(root / "data")
                conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + sql)
                before = dump(conn)
                artifacts = root / "artifacts"
                artifacts.mkdir()
                prepare(artifacts, name)
                # The raw text, as a CLI argument arrives; each export builds its own Path.
                target = relative if relative.startswith("/") else f"{artifacts}/{relative}"
                received = []
                reader = None
                if name == "fifo":
                    def read():
                        with open(target, "rb") as stream:
                            received.append(stream.read().decode("utf-8"))
                    reader = threading.Thread(target=read, daemon=True)
                    reader.start()
                try:
                    result = run(conn, command, args, target)
                    conn.commit()
                    expected = {"ok": result}
                except Exception as exc:  # CLI's observable error projection
                    conn.rollback()
                    expected = {"error": dict(code=exc.code if isinstance(exc, AppError) else "internal_error",
                        message=str(exc), hint=getattr(exc, "hint", None), details=getattr(exc, "details", None),
                        retryable=getattr(exc, "retryable", False))}
                if reader is not None:
                    reader.join(timeout=30)
                    assert not reader.is_alive()
                after = dump(conn)
                conn.close()
                if (artifacts / "locked").exists():
                    (artifacts / "locked").chmod(0o700)
                files = {}
                for path in sorted(artifacts.rglob("*")):
                    key = str(path.relative_to(artifacts))
                    if path.is_symlink():
                        files[key] = {"symlink": os.readlink(path)}
                    elif path.is_dir():
                        files[key] = {"directory": True}
                    elif path.is_file():
                        files[key] = {"text": path.read_bytes().decode("utf-8")}
                    else:
                        files[key] = {"special": True}
                cases.append(dict(
                    name=f"{command}:{name}", command=command, args=args, target=name, sql=sql,
                    expected=json.loads(json.dumps(expected).replace(root_text, "<root>")),
                    changed={table: rows for table, rows in after.items() if before.get(table) != rows},
                    files=files, received=received,
                ))
    return json.dumps(cases, ensure_ascii=True, indent=2) + "\n"


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "CSV write vectors drifted"
    else:
        OUT.write_text(content)
