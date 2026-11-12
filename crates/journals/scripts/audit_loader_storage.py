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
from kassiber.errors import AppError
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


def projection_outcome(snapshot):
    from kassiber.core.custody_compute import compute_custody_projection
    from tests.custody_snapshot import to_jsonable
    try:
        projection = compute_custody_projection(snapshot).finalized_tax_projection
    except Exception as error:
        return {"boundaries": exception_boundaries(error)}
    return {"value": to_jsonable(projection, redact_config=False)}


def expected(conn, profile, *, project=False):
    try:
        snapshot = load_custody_snapshot(conn, profile, as_of=cases.AS_OF)
    except Exception as error:
        return {"phase": "load", "refusal": {"family": type(error).__name__, "code": getattr(error, "code", None),
                            "message": str(error)}, "boundaries": exception_boundaries(error)}
    projection = {"projection": projection_outcome(snapshot)} if project else {}
    try:
        return {"phase": "v1", "v1": json.dumps(snapshot, ensure_ascii=True, allow_nan=True),
                "snapshot": snapshot, **projection}
    except Exception as error:
        return {"phase": "v1", "refusal": {"family": type(error).__name__, "code": getattr(error, "code", None),
                            "message": str(error)}, "boundaries": exception_boundaries(error),
                **projection}


def boundaries_match(reference, actual):
    # Both boundary code/message pairs must match exactly, including refusals.
    return actual == reference


def classify(reference, actual):
    if "refusal" in reference:
        if "v1" in actual:
            return "native_admitted_refused_value"
        return "equal_refusal" if boundaries_match(reference["boundaries"], actual.get("boundaries")) else "different_refusal"
    if actual.get("v1") == reference["v1"]:
        return "equal_value"
    return "native_refused_value" if "error" in actual else "different_value"


def classify_projection(reference, actual):
    """No snapshot acceptance or later unrelated refusal substitutes for parity."""
    reference = reference.get("projection", reference)
    actual = actual.get("projection", actual)
    if "value" in reference:
        if "value" not in actual:
            return "native_refused_projection"
        return ("equal_projection" if projection_text(reference["value"]) == projection_text(actual["value"])
                else "different_projection")
    if "value" in actual:
        return "native_admitted_refused_projection"
    return ("equal_projection_refusal" if boundaries_match(reference.get("boundaries"), actual.get("boundaries"))
            else "different_projection_refusal")


REVISION_REFUSAL = "Custody revision references contain incompatible types"
HASH_SEEDS = tuple(str(seed) for seed in range(8))


def mixed_revision_refusal(reference, table, column, stored_type):
    return (table == "custody_components"
            and column in ("supersedes_component_id", "superseded_by_component_id")
            and stored_type in ("integer", "real", "blob")
            and reference.get("phase") == "load"
            and reference.get("refusal") == {
                "family": "AppError", "code": "validation", "message": REVISION_REFUSAL})


def reference_seed_sweep(request):
    """Require one typed refusal from this exact case in eight fresh processes."""
    by_seed = {}
    expected_refusal = {"family": "AppError", "code": "validation", "message": REVISION_REFUSAL}
    expected_boundaries = exception_boundaries(AppError(REVISION_REFUSAL, code="validation"))
    for seed in HASH_SEEDS:
        result = subprocess.run(
            [sys.executable, str(Path(__file__).resolve()), "--reference-worker"],
            input=json.dumps(request, ensure_ascii=True), capture_output=True, text=True,
            env={**os.environ, "PYTHONHASHSEED": seed}, timeout=120, check=True,
        )
        outcome = json.loads(result.stdout)
        if (result.stderr or outcome != {"phase": "load", "refusal": expected_refusal,
                                         "boundaries": expected_boundaries}):
            raise RuntimeError("mixed revision seed probe did not reproduce the exact validation refusal")
        by_seed[seed] = outcome
    return {"status": "exact deterministic validation refusal", "outcomes_by_hash_seed": by_seed}


def reference_worker():
    request = json.load(sys.stdin)
    with sqlite3.connect(":memory:") as conn:
        conn.row_factory = sqlite3.Row
        conn.executescript(request["sql"])
        before = conn.total_changes
        outcome = expected(conn, request["profile"], project=request["project"])
        if conn.total_changes != before:
            raise RuntimeError("Python seed probe wrote rows")
        print(json.dumps({key: outcome[key] for key in ("phase", "refusal", "boundaries") if key in outcome}))
    return 0


def projection_text(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True, allow_nan=False)


def outcome_record(outcome):
    if "value" in outcome:
        text = projection_text(outcome["value"])
        return {"sha256": hashlib.sha256(text.encode("ascii")).hexdigest()}
    return {key: outcome[key] for key in ("boundaries", "stage", "error") if key in outcome}


def summarize(result):
    summary = {key: value for key, value in result.items() if key != "cases"}
    summary["schema"] = "kassiber.custody-loader-storage-audit-summary.v4"
    summary["total_cases"] = len(result["cases"])
    gate = "projection_status" if result.get("projection") else "status"
    summary["gate"] = gate
    summary["unresolved"] = sum(not record[gate].startswith("equal_") for record in result["cases"])
    summary["unresolved_by_storage"] = dict(sorted(Counter(
        record["storage"] for record in result["cases"] if not record[gate].startswith("equal_")
    ).items()))
    if result.get("projection"):
        summary["zero_write_cases"] = sum(record.get("writes") == 0 for record in result["cases"])
    summary["deterministic_revision_cases"] = [
        {key: record[key] for key in ("fixture", "affinity", "table", "column", "rowid", "storage", "python_seed_sweep")}
        for record in result["cases"] if "python_seed_sweep" in record
    ]
    summary["deterministic_revision_count"] = len(summary["deterministic_revision_cases"])
    summary["pending_python_determinism_count"] = 0
    summary["limitations"] = [
        "A predicate/join mutation can exclude the changed row; equality then proves filtering, not downstream scalar handling.",
        "Both declared-affinity and no-affinity variants record actual typeof(); requested classes are never counted as stored classes.",
        "All physical columns of the explicit loader-table manifest are audited; authorizer reads must be a subset.",
        "Every physical row of the small synthetic fixtures is audited, including inactive/foreign rows; this does not exhaust combinations of corrupted cells.",
        "Load and v1 serialization phases are distinguished; native custody admission is recorded separately.",
        "Projection mode compares the native in-process custody stages with Python through finalized projection, including exact CLI/daemon refusal code/message and zero writes. It does not run the unrouted R4 journal command or ledger persistence.",
        "Mixed numeric/TEXT and BLOB/TEXT revision references require one exact typed validation refusal; each such case is independently repeated under eight Python hash seeds. No variant sets are accepted.",
    ]
    failures = {}
    for record in result["cases"]:
        if record[gate].startswith("equal_"):
            continue
        key = (record["fixture"], record["table"], record["column"])
        statuses = failures.setdefault(key, {})
        for field in ("status", "projection_status"):
            if field in record and not record[field].startswith("equal_"):
                statuses.setdefault(record[field], set()).add(record["storage"])
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


def run(binary, out, summary_out=None, *, first_row=False, declared_only=False, project=True, non_blob=False):
    records = []
    inventories = {}
    storage_cases = {key: value for key, value in STORAGE.items()
                     if not non_blob or not key.startswith("blob")}
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
                                        for storage, expression in storage_cases.items():
                                            update = f'UPDATE "{table}" SET "{column}"={expression} WHERE rowid={rowid}'
                                            conn.execute("SAVEPOINT storage_audit")
                                            try:
                                                conn.execute(update)
                                                changed_rows = conn.execute("SELECT changes()").fetchone()[0]
                                                stored_type = conn.execute(
                                                    f'SELECT typeof("{column}") FROM "{table}" WHERE rowid=?', (rowid,)
                                                ).fetchone()[0]
                                                before = conn.total_changes
                                                reference = expected(conn, profile, project=project)
                                                if before != conn.total_changes:
                                                    raise RuntimeError("Python loader/projection wrote rows")
                                            finally:
                                                conn.execute("ROLLBACK TO storage_audit")
                                                conn.execute("RELEASE storage_audit")
                                            request = {"sql": sql + ";\n" + update + ";", "profile": profile,
                                                       "as_of": cases.AS_OF, "project": project}
                                            native.stdin.write(json.dumps(request, ensure_ascii=True) + "\n")
                                            native.stdin.flush()
                                            response = native.stdout.readline()
                                            if not response:
                                                raise RuntimeError("native storage audit probe stopped")
                                            actual = json.loads(response)
                                            if "text" in actual.get("projection", {}):
                                                actual["projection"] = {"value": json.loads(actual["projection"]["text"])}
                                            seed_record = (reference_seed_sweep(request)
                                                           if mixed_revision_refusal(reference, table, column, stored_type) else None)
                                            status = classify(reference, actual)
                                            record = {"fixture": fixture, "affinity": affinity,
                                                      "table": table, "column": column, "rowid": rowid,
                                                      "storage": storage, "stored_type": stored_type,
                                                      "changed_rows": changed_rows, "python_phase": reference["phase"],
                                                      "read_in_baseline": (table, column) in columns,
                                                      "status": status}
                                            if project:
                                                record["projection_status"] = classify_projection(reference, actual)
                                                record["python_projection"] = outcome_record(reference.get("projection", reference))
                                                record["native_projection"] = outcome_record(actual.get("projection", actual))
                                                record["writes"] = actual.get("writes")
                                                if record["writes"] != 0:
                                                    raise RuntimeError("native loader/projection did not prove zero writes")
                                            if "refusal" in reference:
                                                record["python"] = reference["refusal"]
                                                record["python_boundaries"] = reference["boundaries"]
                                            else:
                                                record["python_v1_sha256"] = hashlib.sha256(reference["v1"].encode("ascii")).hexdigest()
                                            if seed_record:
                                                record["python_seed_sweep"] = seed_record
                                            if "refusal" in actual:
                                                record["native"] = actual["refusal"]
                                                record["native_boundaries"] = actual.get("boundaries")
                                            elif "error" in actual:
                                                record["native_error"] = actual["error"]
                                            elif "v1" in actual:
                                                record["native_v1_sha256"] = hashlib.sha256(actual["v1"].encode("ascii")).hexdigest()
                                            if "adapter_error" in actual:
                                                record["native_admission_error"] = actual["adapter_error"]
                                            if status == "native_refused_value" and not project:
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
    result = {"schema": "kassiber.custody-loader-storage-audit.v4", "as_of": cases.AS_OF,
              "projection": project,
              "python_hash_seed": os.environ.get("PYTHONHASHSEED"),
              "row_selection": "first physical row" if first_row else "every physical row in the synthetic fixtures",
              "affinities": ["declared"] if declared_only else ["declared", "none"],
              "loaded_tables": list(LOADED_TABLES),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "storage": storage_cases, "read_columns": inventories,
              "excluded_storage": [key for key in STORAGE if key not in storage_cases],
              "summary": dict(sorted(Counter(record["status"] for record in records).items())),
              "populated_summary": dict(sorted(Counter(record["status"] for record in records if record["changed_rows"]).items())),
              "empty_table_cases": sum(not record["changed_rows"] for record in records),
              "cases": records}
    if project:
        result["projection_summary"] = dict(sorted(Counter(record["projection_status"] for record in records).items()))
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
    if project:
        print(json.dumps(result["projection_summary"], sort_keys=True))
    gate = "projection_status" if project else "status"
    return 0 if not inventoried - populated and all(record[gate].startswith("equal_") for record in records) else 1


def main():
    if sys.argv[1:] == ["--reference-worker"]:
        return reference_worker()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--summary-out", type=Path)
    parser.add_argument("--first-row", action="store_true", help="reproduce the historical read-column/first-row audit")
    parser.add_argument("--declared-only", action="store_true", help="skip the no-affinity storage variant")
    parser.add_argument("--snapshot-only", action="store_true", help="historical v1 transport audit without native projection")
    parser.add_argument("--non-blob", action="store_true", help="rerun the non-BLOB storage-class regression subset")
    args = parser.parse_args()
    args.out.with_suffix(".jsonl").write_text("")
    return run(args.binary.resolve(), args.out, args.summary_out,
               first_row=args.first_row, declared_only=args.declared_only, project=not args.snapshot_only,
               non_blob=args.non_blob)


if __name__ == "__main__":
    raise SystemExit(main())
