#!/usr/bin/env python3
"""Audit synthetic SQLite storage classes against the Python loader oracle.

Set TMPDIR inside the worktree's target directory. The authorizer inventory is
fixture-dependent; changing a predicate column can exclude the selected row.
This is a gap inventory, not proof of lossless storage support.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.custody_snapshot import load_custody_snapshot
from kassiber.db import open_db
from tests import test_rust_custody_loader as cases
from tests.test_rp2_ownership_transfers import OwnershipDeriverHandlerTest

STORAGE = {
    "null": "NULL",
    "integer": "7",
    "real": "1.5",
    "text": "'legacy scalar'",
    "empty_text": "''",
    "blob": "x'ff00'",
    "positive_infinity": "1e999",
    "negative_infinity": "-1e999",
    "invalid_utf8": "CAST(x'ff00' AS TEXT)",
}


def expected(conn, profile):
    try:
        snapshot = load_custody_snapshot(conn, profile, as_of=cases.AS_OF)
    except Exception as error:
        return {"phase": "load", "refusal": {"family": type(error).__name__, "code": getattr(error, "code", None),
                            "message": str(error)}}
    try:
        return {"phase": "v1", "v1": json.dumps(snapshot, ensure_ascii=True, allow_nan=True)}
    except Exception as error:
        return {"phase": "v1", "refusal": {"family": type(error).__name__, "code": getattr(error, "code", None),
                            "message": str(error)}}


def classify(reference, actual):
    if "refusal" in reference:
        refusal = actual.get("refusal", {})
        return "equal_refusal" if all(refusal.get(key) == value for key, value in reference["refusal"].items()) else "different_refusal"
    if actual.get("v1") == reference["v1"]:
        return "equal_value"
    return "native_refused_value" if "error" in actual else "different_value"


def summarize(result):
    summary = {key: value for key, value in result.items() if key != "cases"}
    summary["schema"] = "kassiber.custody-loader-storage-audit-summary.v1"
    summary["total_cases"] = len(result["cases"])
    summary["limitations"] = [
        "Only columns read by these two synthetic fixtures are inventoried.",
        "The first physical row can be excluded by a changed predicate or join column.",
        "SQLite affinity can coerce the requested storage class; full records retain typeof().",
        "Empty-table cases are not storage-class coverage.",
        "The loader and JSON serialization phases are distinguished; custody admission is recorded separately.",
    ]
    failures = {}
    for record in result["cases"]:
        if record["status"].startswith("equal_"):
            continue
        key = (record["fixture"], record["table"], record["column"])
        statuses = failures.setdefault(key, {})
        statuses.setdefault(record["status"], []).append(record["storage"])
    summary["failures_by_column"] = [
        {"fixture": fixture, "table": table, "column": column, **statuses}
        for (fixture, table, column), statuses in sorted(failures.items())
    ]
    return summary


def run(binary, out, summary_out=None):
    records = []
    inventories = {}
    with tempfile.TemporaryDirectory(prefix="loader-storage-audit-") as directory:
        source = open_db(Path(directory))
        helper = OwnershipDeriverHandlerTest()
        helper._seed(source)
        try:
            for fixture, seed in (
                ("structural", lambda book: cases.test_loader_covers_every_structural_table_column_and_filter(book)),
                ("sources", lambda book: cases.test_loader_receipt_index_preserves_sources_counts_and_profile_scope(book, False)),
            ):
                book = cases.loose_book.__wrapped__((source, helper))
                conn, local_helper = next(book)
                try:
                    with patch.object(cases, "check", lambda _conn: None):
                        seed((conn, local_helper))
                    if fixture == "sources":
                        cases.insert(conn, "freshness_source_states", "checkpoint",
                                     source_key="onchain_wallet:wallet-a", checkpoint_json='{"highest_used":{"0":0}}')
                    profile = dict(cases._profile(conn))
                    columns = set()

                    def authorize(action, table, column, _database, _trigger):
                        if action == sqlite3.SQLITE_READ and table and column and not table.startswith("sqlite_"):
                            columns.add((table, column))
                        return sqlite3.SQLITE_OK

                    conn.set_authorizer(authorize)
                    baseline = expected(conn, profile)
                    conn.set_authorizer(None)
                    if "refusal" in baseline:
                        raise RuntimeError(f"{fixture}: baseline must load and serialize")
                    sql = "\n".join(conn.iterdump())
                    inventories[fixture] = [list(item) for item in sorted(columns)]
                    with subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                          stderr=subprocess.PIPE, text=True) as native:
                        for table, column in sorted(columns):
                            for storage, expression in STORAGE.items():
                                update = (f'UPDATE "{table}" SET "{column}"={expression} '
                                          f'WHERE rowid=(SELECT MIN(rowid) FROM "{table}")')
                                conn.execute("SAVEPOINT storage_audit")
                                try:
                                    conn.execute(update)
                                    changed_rows = conn.execute("SELECT changes()").fetchone()[0]
                                    stored_type = conn.execute(
                                        f'SELECT typeof("{column}") FROM "{table}" ORDER BY rowid LIMIT 1'
                                    ).fetchone()
                                    reference = expected(conn, profile)
                                finally:
                                    conn.execute("ROLLBACK TO storage_audit")
                                    conn.execute("RELEASE storage_audit")
                                request = {"sql": sql + ";\n" + update + ";", "profile": profile, "as_of": cases.AS_OF}
                                native.stdin.write(json.dumps(request, ensure_ascii=True) + "\n")
                                native.stdin.flush()
                                response = native.stdout.readline()
                                if not response:
                                    raise RuntimeError("native storage audit probe stopped")
                                actual = json.loads(response)
                                status = classify(reference, actual)
                                record = {"fixture": fixture, "table": table, "column": column,
                                          "storage": storage, "stored_type": stored_type[0] if stored_type else None,
                                          "changed_rows": changed_rows, "python_phase": reference["phase"],
                                          "status": status}
                                if "refusal" in reference:
                                    record["python"] = reference["refusal"]
                                else:
                                    record["python_v1_sha256"] = hashlib.sha256(reference["v1"].encode("ascii")).hexdigest()
                                if "refusal" in actual:
                                    record["native"] = {key: actual["refusal"].get(key) for key in ("family", "code", "message")}
                                elif "error" in actual:
                                    record["native_error"] = actual["error"]
                                elif "v1" in actual:
                                    record["native_v1_sha256"] = hashlib.sha256(actual["v1"].encode("ascii")).hexdigest()
                                if "adapter_error" in actual:
                                    record["native_admission_error"] = actual["adapter_error"]
                                records.append(record)
                        native.stdin.close()
                        if native.wait() or native.stderr.read():
                            raise RuntimeError("native storage audit probe failed")
                    print(f"{fixture}: {len(columns)} read columns audited", flush=True)
                finally:
                    book.close()
        finally:
            source.close()
    result = {"schema": "kassiber.custody-loader-storage-audit.v1", "as_of": cases.AS_OF,
              "python_hash_seed": os.environ.get("PYTHONHASHSEED"),
              "row_selection": "first physical row in each synthetic table",
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "storage": STORAGE, "read_columns": inventories,
              "summary": dict(sorted(Counter(record["status"] for record in records).items())),
              "populated_summary": dict(sorted(Counter(record["status"] for record in records if record["changed_rows"]).items())),
              "empty_table_cases": sum(not record["changed_rows"] for record in records),
              "cases": records}
    out.write_text(json.dumps(result, ensure_ascii=True, indent=2) + "\n")
    if summary_out is not None:
        summary_out.write_text(json.dumps(summarize(result), ensure_ascii=True, indent=2) + "\n")
    print(json.dumps(result["summary"], sort_keys=True))
    return 0 if all(record["status"].startswith("equal_") for record in records) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--summary-out", type=Path)
    args = parser.parse_args()
    return run(args.binary.resolve(), args.out, args.summary_out)


if __name__ == "__main__":
    raise SystemExit(main())
