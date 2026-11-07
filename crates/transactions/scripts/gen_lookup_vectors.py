#!/usr/bin/env python3
"""Freeze CPython's transaction-reference resolution for the Rust lookup.

Each reference runs through `core.transaction_lookup.find_transaction` (with
and without a direction), `cli.handlers.resolve_transaction` and the component
planner's `_resolve_transaction` on one synthetic book. The SQL that builds the
book is part of the vectors, so both sides start from the same rows.

    uv run --locked python crates/transactions/scripts/gen_lookup_vectors.py [--check]
"""
import argparse
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import resolve_transaction
from kassiber.core.custody_component_planner import _resolve_transaction as planner_resolve
from kassiber.core.transaction_lookup import find_transaction
from kassiber.core.ui_snapshot import build_transactions_resolve_snapshot
from kassiber.db import open_db
from kassiber.envelope import build_error_envelope
from kassiber.errors import AppError

PATH = ROOT / "crates/transactions/tests/data/lookup-vectors.json"
NOW = "2026-01-01T00:00:00Z"
SHARED = "a" * 64
MANY = "b" * 64
LABELED = "c" * 64


def statements():
    sql = [
        f"INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','{NOW}')",
        f"INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book','{NOW}')",
        f"INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('other','ws','Other','{NOW}')",
    ]
    wallets = [
        ("wallet-a", "profile", "Mixer Out"),
        ("wallet-b", "profile", "Cold B"),
        ("wallet-c", "profile", "Cold C"),
        ("wallet-d", "profile", "Straße"),
        ("wallet-in", "profile", "in"),
        ("wal:x", "profile", "Colon Wallet"),
        ("wallet-o", "other", "Cold B"),
    ]
    for wallet_id, profile, label in wallets:
        sql.append(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
            f"VALUES('{wallet_id}','ws','{profile}','{label}','custom','{{}}','{NOW}')"
        )
    rows = [
        ("tx-a-out", "profile", "wallet-a", SHARED, "outbound", 300_000, 1_000, 1),
        ("tx-b-in", "profile", "wallet-b", SHARED, "inbound", 100_000, 0, 2),
        ("tx-c-in", "profile", "wallet-c", SHARED, "inbound", 199_000, 0, 3),
        ("tx-colon", "profile", "wallet-a", "kraken:L123", "inbound", 5_000, 0, 4),
        ("tx-dup-1", "profile", "wallet-a", "dup", "outbound", 7, 0, 5),
        ("tx-dup-2", "profile", "wallet-b", "dup", "inbound", 6, 0, 6),
        ("tx-dup-colon-1", "profile", "wallet-a", "dup:1", "outbound", 9, 0, 7),
        ("tx-dup-colon-2", "profile", "wallet-b", "dup:1", "inbound", 8, 0, 8),
        ("5", "profile", "wallet-a", "five", "inbound", 5, 0, 9),
        ("True", "profile", "wallet-a", None, "inbound", 1, 0, 9),
        ("False", "profile", "wallet-a", None, "inbound", 2, 0, 9),
        ("0", "profile", "wallet-a", None, "inbound", 3, 0, 9),
        ("[]", "profile", "wallet-a", None, "inbound", 4, 0, 9),
        ("tx-unicode", "profile", "wallet-d", "ünï", "inbound", 11, 0, 9),
        ("tx-l-strasse", "profile", "wallet-d", LABELED, "inbound", 12, 0, 10),
        ("tx-l-in", "profile", "wallet-in", LABELED, "inbound", 13, 0, 11),
        ("tx-l-colon", "profile", "wal:x", LABELED, "outbound", 14, 0, 12),
        ("tx-other", "other", "wallet-o", SHARED, "inbound", 15, 0, 13),
    ]
    rows += [
        (f"tx-many-{i:02d}", "profile", "wallet-a" if i % 3 else "wallet-b", MANY,
         "outbound" if i % 2 else "inbound", 1_000 + i, i, 20 + i)
        for i in range(25)
    ]
    for tx_id, profile, wallet, external, direction, amount, fee, minute in rows:
        external_sql = "NULL" if external is None else f"'{external}'"
        sql.append(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,"
            "external_id,occurred_at,direction,asset,amount,fee,created_at) VALUES("
            f"'{tx_id}','ws','{profile}','{wallet}','fp-{tx_id}',{external_sql},"
            f"'2026-01-01T{minute // 60:02d}:{minute % 60:02d}:00Z','{direction}','BTC',"
            f"{amount},{fee},'{NOW}')"
        )
    return sql


REFERENCES = [
    "tx-b-in", " tx-b-in ", "\ttx-b-in　", "kraken:L123", "kraken:L123:in",
    "kraken:L123:out", SHARED[:12], "missing", "", "   ", None, 0, 5, 5.0,
    True, False, [], ["tx-b-in"], {}, {"a": 1}, "five", "ünï", "ÜNÏ",
    SHARED, f"{SHARED}:out", f"{SHARED}:OUT", f"{SHARED}:in", f"{SHARED}:inbound",
    f"{SHARED}:cold b", f"{SHARED}:COLD B", f"{SHARED}:wallet-c:in",
    f"{SHARED}:Cold C:inbound", f"{SHARED}:Cold B:out", f"{SHARED} : cold b ",
    f"{SHARED}:", f":{SHARED}", f"{SHARED}: in", f"{SHARED}:cold b: in",
    f"{SHARED}:wallet-o", f"{SHARED}:nope", f"{SHARED}:wallet-b:IN",
    "dup", "dup:1", "dup:out", "dup:1:out", "dup:1:in", "dup:wallet-b",
    MANY, f"{MANY}:in", f"{MANY}:out", f"{MANY}:wallet-b", f"{MANY}:wallet-b:out",
    LABELED, f"{LABELED}:in", f"{LABELED}:strasse", f"{LABELED}:STRASSE",
    f"{LABELED}:wal:x", f"{LABELED}:wal:x:out", f"{LABELED}:wal:x:in",
    f"{LABELED}:wal", "tx-other", "\ud800", " x\udc80\udc81", f"{SHARED}:\ud800",
]


SNAPSHOT_QUERIES = [
    SHARED, f"{SHARED}:Cold B", f"{SHARED}:in", f"{SHARED}:nope", MANY, f"{MANY}:wallet-b",
    "kraken:L123", "tx-colon", "dup:1:in", "TX-COLON", f"{LABELED}:in", "five",
]


def snapshot(conn, query):
    try:
        return build_transactions_resolve_snapshot(conn, {"query": query})
    except AppError as exc:
        return build_error_envelope(
            exc.code, str(exc), details=exc.details, hint=exc.hint, retryable=exc.retryable
        )


def outcome(call):
    try:
        row = call()
    except AppError as exc:
        return {
            "error": {
                "code": exc.code,
                "message": str(exc),
                "hint": exc.hint,
                "details": exc.details,
            }
        }
    except UnicodeEncodeError as exc:
        return {"error": {"code": "internal_error", "message": str(exc), "hint": None, "details": None}}
    return {"id": None if row is None else row["id"]}


def content():
    assert sys.version_info[:2] == (3, 11), "use locked Python 3.11"
    sql = statements()
    with tempfile.TemporaryDirectory(prefix="kassiber-lookup-vectors-") as tmp:
        conn = open_db(Path(tmp) / "data")
        try:
            for statement in sql:
                conn.execute(statement)
            conn.execute(
                "INSERT INTO settings(key,value) VALUES"
                "('context_workspace','ws'),('context_profile','profile')"
            )
            conn.commit()
            snapshots = [
                {"query": query, "expected": snapshot(conn, query)} for query in SNAPSHOT_QUERIES
            ]
            cases = []
            for reference in REFERENCES:
                case = {"reference": reference}
                for direction in (None, "outbound", "inbound"):
                    key = direction or "any"
                    case[f"find_{key}"] = outcome(
                        lambda: find_transaction(conn, "profile", reference, direction=direction)
                    )
                    case[f"resolve_{key}"] = outcome(
                        lambda: resolve_transaction(conn, "profile", reference, direction=direction)
                    )
                case["planner"] = outcome(lambda: planner_resolve(conn, "profile", reference))
                cases.append(case)
        finally:
            conn.close()
    return json.dumps(
        {"sql": sql, "cases": cases, "snapshots": snapshots}, ensure_ascii=True, indent=1
    ) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = content()
    if args.check:
        if PATH.read_text() != generated:
            raise SystemExit("lookup vectors drifted")
    else:
        PATH.parent.mkdir(parents=True, exist_ok=True)
        PATH.write_text(generated)
