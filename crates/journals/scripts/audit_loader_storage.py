#!/usr/bin/env python3
"""Audit synthetic SQLite storage classes against the Python loader oracle.

Set TMPDIR inside the worktree's target directory. Audit every physical column
of the loader's tables, including columns discarded by projections, and every
physical row in the synthetic fixtures. Repeat with no declared affinities so
SQLite coercion cannot masquerade as scalar coverage. The manifest is explicit:
new loader tables must be added here. Predicate changes may exclude a row; that
outcome is recorded rather than claimed as downstream scalar coverage.
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
from tests.rust_port.custody_loader_oracle import exception_boundaries

LOADED_TABLES = (
    "accounts", "backends", "book_network_bindings", "transactions", "wallets",
    "chain_analysis_observations", "chain_observer_instances",
    "chain_observation_provenance", "wallet_policy_epochs", "wallet_policy_sources",
    "wallet_utxos", "custody_components", "custody_component_legs",
    "custody_component_allocations", "custody_component_economic_terms",
    "custody_component_evidence_commitments", "lightning_node_records", "loan_legs",
    "custody_authored_evidence_snapshots", "custody_authored_migration_issues",
    "custody_gap_reviews", "transaction_pair_dismissals", "freshness_source_states",
)

STORAGE = {
    "null": "NULL",
    "integer": "7",
    "real": "1.5",
    "text": "'legacy scalar'",
    "empty_text": "''",
    "blob": "x'ff00'",
    "blob_integer": "x'3737'",
    "blob_json": "x'7b22636861696e223a22626974636f696e222c226e6574776f726b223a226d61696e227d'",
    "positive_infinity": "1e999",
    "negative_infinity": "-1e999",
    "invalid_utf8": "CAST(x'ff00' AS TEXT)",
}


def expected(conn, profile):
    try:
        snapshot = load_custody_snapshot(conn, profile, as_of=cases.AS_OF)
    except Exception as error:
        return {"phase": "load", "refusal": {"family": type(error).__name__, "code": getattr(error, "code", None),
                            "message": str(error)}, "boundaries": exception_boundaries(error)}
    try:
        return {"phase": "v1", "v1": json.dumps(snapshot, ensure_ascii=True, allow_nan=True),
                "snapshot": snapshot}
    except Exception as error:
        return {"phase": "v1", "refusal": {"family": type(error).__name__, "code": getattr(error, "code", None),
                            "message": str(error)}, "boundaries": exception_boundaries(error)}


def classify(reference, actual):
    if "refusal" in reference:
        if "v1" in actual:
            return "native_admitted_refused_value"
        return "equal_refusal" if actual.get("boundaries") == reference["boundaries"] else "different_refusal"
    if actual.get("v1") == reference["v1"]:
        return "equal_value"
    return "native_refused_value" if "error" in actual else "different_value"


def summarize(result):
    summary = {key: value for key, value in result.items() if key != "cases"}
    summary["schema"] = "kassiber.custody-loader-storage-audit-summary.v2"
    summary["total_cases"] = len(result["cases"])
    summary["limitations"] = [
        "A predicate/join mutation can exclude the changed row; equality then proves filtering, not downstream scalar handling.",
        "Both declared-affinity and no-affinity variants record actual typeof(); requested classes are never counted as stored classes.",
        "All physical columns of the explicit loader-table manifest are audited; authorizer reads must be a subset.",
        "Every physical row of the small synthetic fixtures is audited, including inactive/foreign rows; this does not exhaust combinations of corrupted cells.",
        "Load and v1 serialization phases are distinguished; native custody admission is recorded separately.",
    ]
    failures = {}
    for record in result["cases"]:
        if record["status"].startswith("equal_"):
            continue
        key = (record["fixture"], record["table"], record["column"])
        statuses = failures.setdefault(key, {})
        statuses.setdefault(record["status"], set()).add(record["storage"])
    summary["failures_by_column"] = [
        {"fixture": fixture, "table": table, "column": column,
         **{status: sorted(storage) for status, storage in statuses.items()}}
        for (fixture, table, column), statuses in sorted(failures.items())
    ]
    return summary


def without_affinity(source):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    for (table,) in source.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"):
        names = [row["name"] for row in source.execute(f'PRAGMA table_info("{table}")')]
        fields = ",".join('"' + name + '"' for name in names)
        conn.execute(f'CREATE TABLE "{table}" ({fields})')
        rows = source.execute(f'SELECT {fields} FROM "{table}"').fetchall()
        conn.executemany(f'INSERT INTO "{table}" VALUES ({",".join("?" for _ in names)})', rows)
    return conn


def run(binary, out, summary_out=None, *, first_row=False, declared_only=False):
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
                original, local_helper = next(book)
                conn = original
                try:
                    with patch.object(cases, "check", lambda _conn: None):
                        seed((conn, local_helper))
                    if fixture == "sources":
                        cases.insert(conn, "freshness_source_states", "checkpoint",
                                     source_key="onchain_wallet:wallet-a", checkpoint_json='{"highest_used":{"0":0}}')
                    for table in ("transactions", "custody_components"):
                        conn.execute(f'ALTER TABLE "{table}" ADD COLUMN "_closed_confirmation_observation"')
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
                    unexpected = {table for table, _column in columns} - set(LOADED_TABLES)
                    if unexpected:
                        raise RuntimeError(f"loader manifest omits read tables: {sorted(unexpected)}")
                    physical = {
                        (table, row["name"])
                        for table in LOADED_TABLES
                        for row in conn.execute(f'PRAGMA table_info("{table}")')
                    }
                    if not columns <= physical:
                        raise RuntimeError("authorizer columns missing from physical inventory")
                    inventories[fixture] = {
                        "read_columns": [list(item) for item in sorted(columns)],
                        "physical_columns": [list(item) for item in sorted(physical)],
                    }
                    for affinity in ("declared",) if declared_only else ("declared", "none"):
                        if affinity == "none":
                            conn = without_affinity(original)
                        try:
                            sql = "\n".join(conn.iterdump())
                            if "refusal" in expected(conn, profile):
                                raise RuntimeError(f"{fixture}/{affinity}: baseline refuses")
                            with subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                                  stderr=subprocess.PIPE, text=True) as native:
                                for table, column in sorted(columns if first_row else physical):
                                    rowids = [row[0] for row in conn.execute(f'SELECT rowid FROM "{table}" ORDER BY rowid')]
                                    if first_row:
                                        rowids = rowids[:1]
                                    if not rowids:
                                        continue
                                    for rowid in rowids:
                                        for storage, expression in STORAGE.items():
                                            update = f'UPDATE "{table}" SET "{column}"={expression} WHERE rowid={rowid}'
                                            conn.execute("SAVEPOINT storage_audit")
                                            try:
                                                conn.execute(update)
                                                changed_rows = conn.execute("SELECT changes()").fetchone()[0]
                                                stored_type = conn.execute(
                                                    f'SELECT typeof("{column}") FROM "{table}" WHERE rowid=?', (rowid,)
                                                ).fetchone()[0]
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
                                            record = {"fixture": fixture, "affinity": affinity,
                                                      "table": table, "column": column, "rowid": rowid,
                                                      "storage": storage, "stored_type": stored_type,
                                                      "changed_rows": changed_rows, "python_phase": reference["phase"],
                                                      "read_in_baseline": (table, column) in columns,
                                                      "status": status}
                                            if "refusal" in reference:
                                                record["python"] = reference["refusal"]
                                                record["python_boundaries"] = reference["boundaries"]
                                            else:
                                                record["python_v1_sha256"] = hashlib.sha256(reference["v1"].encode("ascii")).hexdigest()
                                            if "refusal" in actual:
                                                record["native"] = actual["refusal"]
                                                record["native_boundaries"] = actual.get("boundaries")
                                            elif "error" in actual:
                                                record["native_error"] = actual["error"]
                                            elif "v1" in actual:
                                                record["native_v1_sha256"] = hashlib.sha256(actual["v1"].encode("ascii")).hexdigest()
                                            if "adapter_error" in actual:
                                                record["native_admission_error"] = actual["adapter_error"]
                                            if status == "native_refused_value":
                                                from kassiber.core.custody_compute import compute_custody_projection
                                                try:
                                                    compute_custody_projection(reference["snapshot"])
                                                except Exception as error:
                                                    boundary = exception_boundaries(error)
                                                    record["python_downstream_refusal"] = boundary
                                                    if boundary == actual.get("boundaries"):
                                                        record["status"] = "equal_downstream_refusal"
                                                else:
                                                    record["python_downstream"] = "accepted"
                                            records.append(record)
                                            with out.with_suffix(".jsonl").open("a") as progress:
                                                progress.write(json.dumps(record, ensure_ascii=True) + "\n")
                                native.stdin.close()
                                if native.wait() or native.stderr.read():
                                    raise RuntimeError("native storage audit probe failed")
                            print(f"{fixture}/{affinity}: {len(records)} cumulative scalar cases", flush=True)
                        finally:
                            if conn is not original:
                                conn.close()
                                conn = original
                finally:
                    book.close()
        finally:
            source.close()
    result = {"schema": "kassiber.custody-loader-storage-audit.v2", "as_of": cases.AS_OF,
              "python_hash_seed": os.environ.get("PYTHONHASHSEED"),
              "row_selection": "first physical row" if first_row else "every physical row in the synthetic fixtures",
              "affinities": ["declared"] if declared_only else ["declared", "none"],
              "loaded_tables": list(LOADED_TABLES),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "storage": STORAGE, "read_columns": inventories,
              "summary": dict(sorted(Counter(record["status"] for record in records).items())),
              "populated_summary": dict(sorted(Counter(record["status"] for record in records if record["changed_rows"]).items())),
              "empty_table_cases": sum(not record["changed_rows"] for record in records),
              "cases": records}
    inventoried = {tuple(column) for fixture in inventories.values() for column in fixture["physical_columns"]}
    populated = {(record["table"], record["column"]) for record in records}
    result["physical_column_coverage"] = {
        "inventoried": len(inventoried), "populated": len(populated),
        "missing": [list(column) for column in sorted(inventoried - populated)],
    }
    result["storage_class_counts"] = dict(sorted(Counter(record["stored_type"] for record in records).items()))
    out.write_text(json.dumps(result, ensure_ascii=True, indent=2) + "\n")
    if summary_out is not None:
        summary_out.write_text(json.dumps(summarize(result), ensure_ascii=True, indent=2) + "\n")
    print(json.dumps(result["summary"], sort_keys=True))
    return 0 if not inventoried - populated and all(record["status"].startswith("equal_") for record in records) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--summary-out", type=Path)
    parser.add_argument("--first-row", action="store_true", help="reproduce the historical read-column/first-row audit")
    parser.add_argument("--declared-only", action="store_true", help="skip the no-affinity storage variant")
    args = parser.parse_args()
    args.out.with_suffix(".jsonl").write_text("")
    return run(args.binary.resolve(), args.out, args.summary_out,
               first_row=args.first_row, declared_only=args.declared_only)


if __name__ == "__main__":
    raise SystemExit(main())
