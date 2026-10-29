#!/usr/bin/env python3
"""Observer storage oracle only; this is not full post-sync book parity."""
import argparse
import json
from pathlib import Path
import sqlite3
import sys
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber.db import SCHEMA
from kassiber.errors import AppError
from kassiber.core.chain_observer.identity import ObserverIdentity
from kassiber.core.chain_observer.store import CoveragePoint, persist_observer_state

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_observer_persistence.json"
IDENTITY = ObserverIdentity("observer", "workspace", "profile", "wallet", "wallet",
                            "descriptor:default", "bdk", "bitcoin", "bitcoin", ("receive", "change"))


def operations():
    base = {"state": {"z": "é", "a": [1, True, None]}, "coverage": [
        {"branch_key": "receive", "highest_used": 1, "scanned_to": 22, "details": {"z": 2, "a": 1}},
        {"branch_key": "change", "highest_used": None, "scanned_to": 20, "details": {}}],
        "expected_epoch": None}
    yield "insert", base
    yield "duplicate-insert", base
    for epoch in (0, 2):
        yield f"stale-{epoch}", {**base, "expected_epoch": epoch}
    for field, value in (("branch_key", "unknown"), ("branch_key", "change"),
                         ("highest_used", -1), ("scanned_to", -1),
                         ("highest_used", 22), ("highest_used", 23)):
        yield f"invalid-{field}-{value}", {**base, "expected_epoch": 1,
            "coverage": [{**base["coverage"][0], field: value}, base["coverage"][1]]}
    yield "invalid-state", {**base, "state": [], "expected_epoch": 1}
    yield "nonfinite-nested-state", {**base, "state": {}, "raw_state": '{"nested":[1e9999]}', "expected_epoch": 1}
    yield "default-details", {**base, "expected_epoch": 1, "coverage": [
        {"branch_key": "receive", "scanned_to": 1, "highest_used": 0, "details": None}]}
    yield "zero-boundary", {**base, "expected_epoch": 2, "coverage": [
        {"branch_key": "receive", "scanned_to": 1, "highest_used": 0, "details": {}},
        {"branch_key": "change", "scanned_to": 0, "highest_used": None, "details": {}}]}
    yield "replace-empty", {**base, "expected_epoch": 3, "state": {"replaced": True}, "coverage": []}


def vectors():
    conn = sqlite3.connect(":memory:")
    conn.executescript(SCHEMA)
    conn.executescript("""
        INSERT INTO workspaces(id,label,created_at) VALUES('workspace','synthetic','fixed');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','workspace','synthetic','fixed');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at)
        VALUES('wallet','workspace','profile','synthetic','descriptor','fixed');
    """)
    result = []
    for name, operation in operations():
        timestamp = f"time-{len(result)}"
        conn.execute("SAVEPOINT wallet_refresh")
        error = None
        try:
            with patch("kassiber.core.chain_observer.store.now_iso", return_value=timestamp):
                persist_observer_state(conn, IDENTITY, json.loads(operation["raw_state"]) if "raw_state" in operation else operation["state"],
                    tuple(CoveragePoint(**point) for point in operation["coverage"]),
                    expected_epoch=operation["expected_epoch"])
        except AppError as exc:
            error = {"code": exc.code, "message": str(exc), "details": exc.details, "hint": exc.hint, "retryable": exc.retryable}
            conn.execute("ROLLBACK TO wallet_refresh")
        conn.execute("RELEASE wallet_refresh")
        snapshot = {table: [list(row) for row in conn.execute(f"SELECT * FROM {table} ORDER BY rowid")]
                    for table in ("chain_observer_instances", "chain_observer_coverage")}
        result.append({"name": name, "input": operation, "timestamp": timestamp, "error": error, "snapshot": snapshot})
    conn.close()
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = json.dumps(vectors(), indent=2, ensure_ascii=False) + "\n"
    if args.check:
        if DEST.read_text() != output:
            raise SystemExit("Observer persistence oracle drift")
    else:
        DEST.write_text(output)
