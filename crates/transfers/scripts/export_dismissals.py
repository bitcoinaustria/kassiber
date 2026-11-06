#!/usr/bin/env python3
"""Freeze CPython's transfer-dismissal services for the Rust port (main sync S6).

`dismiss` marks journals stale; `dismissals list` reads the vetoes back with
the matcher's expiry rule; `dismissals delete` undoes one by id or by its two
(qualified) legs; `candidate_exclusion_counts` explains short suggestion
lists. Clocks and ids are injected, so nothing is masked.

    uv run --locked python crates/transfers/scripts/export_dismissals.py [--check]
"""
from contextlib import ExitStack
import argparse
import dataclasses
import json
from pathlib import Path
import sys
import tempfile
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli import handlers
from kassiber.core import custody_review_terms as terms
from kassiber.core import transfer_matching as matching
from kassiber.db import open_db
from kassiber.errors import AppError
from datetime import datetime, timezone
from tests.test_transfer_matching import _row

PATH = ROOT / "crates/transfers/tests/dismissals.json"
STAMP = "2026-10-30T12:00:00Z"
SHARED = "a" * 64


def statements():
    sql = [
        f"INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','{STAMP}')",
        f"INSERT INTO profiles(id,workspace_id,label,created_at,last_processed_at,last_processed_tx_count,journal_input_version,last_processed_input_version) VALUES('p','w','Book','{STAMP}','{STAMP}',8,4,4)",
        f"INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('q','w','Other','{STAMP}')",
    ]
    for wallet, profile, label in (("a", "p", "Mixer Out"), ("b", "p", "Cold B"), ("c", "q", "Other")):
        sql.append(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
            f"VALUES('{wallet}','w','{profile}','{label}','custom','{{}}','{STAMP}')"
        )
    rows = [
        ("o1", "p", "a", SHARED, "outbound", 300_000, 1),
        ("i1", "p", "b", SHARED, "inbound", 299_000, 2),
        ("i2", "p", "a", SHARED, "inbound", 1_000, 3),
        ("o2", "p", "b", "ext-o2", "outbound", 50_000, 4),
        ("xo", "p", "a", "ext-xo", "outbound", 70_000, 5),
        ("xi", "p", "b", "ext-xi", "inbound", 69_000, 6),
        ("q1", "q", "c", "ext-q1", "inbound", 9, 7),
    ]
    for tx, profile, wallet, external, direction, amount, minute in rows:
        sql.append(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
            "occurred_at,direction,asset,amount,fee,created_at) VALUES("
            f"'{tx}','w','{profile}','{wallet}','fp-{tx}','{external}','2026-10-0{minute}T00:00:00Z',"
            f"'{direction}','BTC',{amount},0,'{STAMP}')"
        )
    dismissals = [
        ("d-perm", "p", "o1", "i1", "reviewed", "2026-10-01T00:00:00Z", None),
        ("d-future", "p", "o1", "i2", None, "2026-10-02T00:00:00Z", "2026-11-30T00:00:00Z"),
        ("d-past", "p", "xo", "xi", "old", "2026-10-02T00:00:00Z", "2026-10-01T00:00:00Z"),
        ("d-equal", "p", "xo", "i1", None, "2026-10-03T00:00:00Z", STAMP),
        ("d-empty", "p", "o2", "i2", None, "2026-10-03T00:00:00Z", ""),
        ("d-bad", "p", "o2", "i1", None, "2026-10-04T00:00:00Z", "soon"),
        ("d-cross", "p", "o2", "q1", None, "2026-10-04T00:00:00Z", None),
        ("d-other", "q", "q1", "q1", None, "2026-10-05T00:00:00Z", None),
    ]
    for ident, profile, out, inbound, reason, created, expires in dismissals:
        reason_sql = "NULL" if reason is None else f"'{reason}'"
        expires_sql = "NULL" if expires is None else f"'{expires}'"
        sql.append(
            "INSERT INTO transaction_pair_dismissals(id,workspace_id,profile_id,out_transaction_id,"
            f"in_transaction_id,reason,created_at,expires_at) VALUES('{ident}','w','{profile}',"
            f"'{out}','{inbound}',{reason_sql},'{created}',{expires_sql})"
        )
    return sql


OPERATIONS = [
    ("suggest", {}),
    ("review_context", {"workspace": "w", "profile": "p"}),
    ("list", {}),
    ("list", {"include_expired": True}),
    ("list", {"limit": 1}),
    ("list", {"limit": 2, "include_expired": True}),
    ("list", {"limit": 500}),
    ("list", {"limit": 5}),
    ("list", {"limit": 7, "include_expired": True}),
    ("list", {"limit": 0}),
    ("list", {"limit": 501}),
    ("list", {"limit": True}),
    ("list", {"limit": "5"}),
    ("list", {"limit": None}),
    ("list", {"limit": 2.0}),
    ("dismiss", {"tx_out": f"{SHARED}:out", "tx_in": f"{SHARED}:cold b", "reason": None, "expires_in_days": 0}),
    ("dismiss", {"tx_out": "xo", "tx_in": f"{SHARED}:Mixer Out", "reason": "same wallet", "expires_in_days": 0}),
    ("dismiss", {"tx_out": SHARED, "tx_in": "xi", "reason": None, "expires_in_days": 0}),
    ("delete", {"dismissal_id": "d-past"}),
    ("delete", {"dismissal_id": "d-past"}),
    ("delete", {"dismissal_id": "d-other"}),
    ("delete", {"dismissal_id": 5}),
    ("delete", {"dismissal_id": "d-bad", "tx_out": "o2"}),
    ("delete", {"dismissal_id": "", "tx_in": "o2"}),
    ("delete", {}),
    ("delete", {"tx_out": f"{SHARED}:out"}),
    ("delete", {"tx_out": SHARED, "tx_in": "i1"}),
    ("delete", {"tx_out": "xo", "tx_in": "xi"}),
    ("delete", {"tx_out": f"{SHARED}:out", "tx_in": f"{SHARED}:Cold B"}),
    ("delete", {"tx_out": "o2", "tx_in": f"{SHARED}:in"}),
    ("list", {"include_expired": True}),
    ("suggest", {}),
    ("review_context", {"workspace": "w", "profile": "p"}),
]


def dump(conn):
    return {
        "transaction_pair_dismissals": [
            dict(row) for row in conn.execute("SELECT * FROM transaction_pair_dismissals ORDER BY id")
        ],
        "profiles": [
            dict(row)
            for row in conn.execute(
                "SELECT id,last_processed_at,last_processed_tx_count,journal_input_version,"
                "ownership_review_counts_json FROM profiles ORDER BY id"
            )
        ],
    }


def run(conn, operation, args):
    if operation == "suggest":
        return handlers.suggest_transfer_candidates(conn, "w", "p")
    if operation == "review_context":
        from kassiber.daemon import _ui_swap_matching_payload_from_conn

        return _ui_swap_matching_payload_from_conn(conn, "ui.transfers.review_context", dict(args))
    if operation == "list":
        return handlers.list_transfer_dismissals(
            conn, "w", "p", include_expired=args.get("include_expired", False),
            limit=args.get("limit", 200),
        )
    if operation == "dismiss":
        return handlers.dismiss_transfer_candidate(
            conn, "w", "p", args["tx_out"], args["tx_in"], reason=args["reason"],
            expires_in_days=args["expires_in_days"],
        )
    return handlers.delete_transfer_dismissal(
        conn, "w", "p", dismissal_id=args.get("dismissal_id"),
        out_ref=args.get("tx_out"), in_ref=args.get("tx_in"),
    )


def services():
    sql = statements()
    cases = []
    with tempfile.TemporaryDirectory(prefix="kassiber-dismissal-vectors-") as root, ExitStack() as stack:
        sequence = 0

        def uuid4():
            nonlocal sequence
            sequence += 1
            return uuid.UUID(int=sequence)

        stack.enter_context(patch.object(terms.uuid, "uuid4", side_effect=uuid4))
        stack.enter_context(patch.object(terms, "now_iso", return_value=STAMP))
        stack.enter_context(patch.object(handlers, "now_iso", return_value=STAMP))

        class Frozen(datetime):
            @classmethod
            def now(cls, tz=None):
                return cls(2026, 10, 30, 12, tzinfo=timezone.utc)

        stack.enter_context(patch.object(matching, "datetime", Frozen))
        conn = open_db(root)
        try:
            for statement in sql:
                conn.execute(statement)
            conn.commit()
            for operation, args in OPERATIONS:
                before = sequence
                case = {"operation": operation, "args": args, "id": str(uuid.UUID(int=before + 1))}
                try:
                    case["result"] = run(conn, operation, args)
                except AppError as exc:
                    conn.rollback()
                    case["error"] = {"code": exc.code, "message": str(exc), "hint": exc.hint, "details": exc.details}
                case["rows"] = dump(conn)
                cases.append(case)
        finally:
            conn.close()
    return sql, cases


def exclusions():
    rows = [
        _row(id="o1", direction="outbound"), _row(id="i1", direction="inbound"),
        _row(id="o2", direction="outbound"), _row(id="i2", direction="inbound"),
        _row(id="o3", direction="outbound"), _row(id="i3", direction="inbound"),
        _row(id="ex", direction="outbound", excluded=1), _row(id="fee", direction="fee"),
    ]
    variants = [
        {},
        {"pair_records": [{"out_transaction_id": "o2", "in_transaction_id": "i2"}]},
        {"pair_records": [{"out_transaction_id": "o2", "in_transaction_id": "i2"},
                          {"out_transaction_id": "o3", "in_transaction_id": "i3", "deleted_at": "x"},
                          {"out_transaction_id": "ex", "in_transaction_id": None}]},
        {"booked_move_transaction_ids": ["i3", "o2", "ghost"]},
        {"pair_records": [{"out_transaction_id": "o2", "in_transaction_id": "i2"}],
         "booked_move_transaction_ids": ["o2", "i3"]},
    ]
    dismissals = [
        {"out_transaction_id": "o1", "in_transaction_id": "i1", "expires_at": None},
        {"out_transaction_id": "o1", "in_transaction_id": "i1", "expires_at": "2027-01-01T00:00:00Z"},
        {"out_transaction_id": "o1", "in_transaction_id": "i2", "expires_at": None},
        {"out_transaction_id": "o3", "in_transaction_id": "i3", "expires_at": ""},
        {"out_transaction_id": "o3", "in_transaction_id": "ex", "expires_at": None},
        {"out_transaction_id": "o3", "in_transaction_id": "i1", "expires_at": STAMP},
        {"out_transaction_id": "o2", "in_transaction_id": "i1", "expires_at": "2026-10-30T12:00:01Z"},
        {"out_transaction_id": "ghost", "in_transaction_id": "i1", "expires_at": None},
        {"out_transaction_id": "fee", "in_transaction_id": "i3", "expires_at": None},
    ]
    cases = []
    for variant in variants:
        for subset in (dismissals, dismissals[:1], []):
            options = dict(variant, dismissals=subset, now_iso=STAMP)
            counts = matching.candidate_exclusion_counts(rows, **options)
            cases.append({"options": options, "result": counts})
    return {"rows": [dict(row) for row in rows], "cases": cases}


def content():
    assert sys.version_info[:2] == (3, 11), "use locked Python 3.11"
    sql, cases = services()
    return json.dumps(
        {"now": STAMP, "sql": sql, "cases": cases, "exclusions": exclusions()},
        ensure_ascii=True, indent=1,
    ) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    generated = content()
    if args.check:
        if PATH.read_text() != generated:
            raise SystemExit("dismissal vectors drifted")
    else:
        PATH.write_text(generated)
