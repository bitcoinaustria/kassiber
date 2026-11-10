#!/usr/bin/env python3
"""Freeze synthetic current-case reads and independent waiting-root witnesses."""
import argparse
import base64
from contextlib import ExitStack
import json
import re
from pathlib import Path
from sqlite3.dump import _iterdump
import sqlite3
import sys
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from export_gap_reads import template
from export_pairs import seed
from kassiber.cli import handlers
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION
from kassiber.core import custody_authored_migration, custody_component_planner, custody_components, custody_journal
from kassiber.core import custody_review_terms, quarantine_review, review_workflow as review
from tests.custody_snapshot import to_jsonable
from tests.rust_port.journal_render_oracle import canonical, capture_build

CLOCK = "2026-10-31T12:00:00Z"
DESTINATION = ROOT / "crates/transfers/tests/review_cases.json"


def build(conn, profile):
    builder = custody_journal.CustodyJournalBuilder(conn, profile, as_of=CLOCK)
    original_load = builder.load_snapshot
    snapshots = []

    def load():
        snapshot = original_load()
        snapshots.append(snapshot)
        return snapshot

    with capture_build(builder) as (record, _), patch.object(
        custody_journal, "build_ledger_state", lambda *_: builder.build(),
    ), patch.object(builder, "load_snapshot", load):
        state = review._build(conn, profile)
    assert len(snapshots) == 1
    return state, {
        "as_of": CLOCK, "profile": dict(profile), "snapshot": snapshots[0],
        "entry_ids": record["entry_ids"], "review": True,
        "expected": {"state": canonical(to_jsonable(state, redact_config=False))},
    }


def token(payload):
    return base64.urlsafe_b64encode(review._json(payload).encode()).decode()


def waiting_vectors():
    def row(reason, at="2020", asset="BTC"):
        return dict(reason=reason, occurred_at=at, asset=asset)

    roots = {"a": row("missing_spot_price", "2019"),
             "b": row("custody_quantity_unresolved", "2018"),
             "same": row("pending_onchain_confirmation", "2019"),
             "eth": row("missing_spot_price", "2019", "ETH")}
    vectors = []
    for reason, field in (("custody_basis_barrier", "root_transaction_ids"),
                          ("transfer_pair_dependency_blocked", "blocked_by_transaction_ids")):
        for candidates in ([], ["missing"], ["a"], ["a", "b"], ["a", "same"],
                           ["middle"], ["end"], ["middle", "a"], ["end", "b"]):
            rows = {**roots, "middle": row(reason), "end": row(reason)}
            details = {"middle": {field: ["a"]}, "end": {field: candidates}}
            vectors.append(dict(name=f"{reason}-{candidates}", rows=rows, details=details))
    for group in ("batch", 7, True, 0, "", [], {}):
        rows = {**roots, "child": row("derived_transfer_group_blocked")}
        details = {"a": {"transfer_group_id": group}, "b": {"transfer_group_id": group},
                   "child": {"transfer_group_id": group}}
        vectors.append(dict(name=f"group-{group!r}", rows=rows, details=details))
    for since in ("2018", "2019", "missing", "", None, 2018, ["2018"]):
        rows = {**roots, "child": row("basis_provenance_incomplete")}
        vectors.append(dict(name=f"uncertain-{since!r}", rows=rows,
                            details={"child": {"lot_state_uncertain_since": since}}))
    vectors += [
        dict(name="cycle", rows={"a": row("custody_basis_barrier"),
                                  "b": row("transfer_pair_dependency_blocked")},
             details={"a": {"root_transaction_ids": ["b"]},
                      "b": {"blocked_by_transaction_ids": ["a"]}}),
        dict(name="unnamed-carry", rows={"a": row("at_swap_basis_carry_unresolved"),
                                          "b": row("bitcoin_rail_carry_basis_unresolved")}, details={}),
        dict(name="numeric-group-equality", rows={"a": row("missing_spot_price"),
                                                   "b": row("derived_transfer_group_blocked")},
             details={"a": {"transfer_group_id": True}, "b": {"transfer_group_id": 1}}),
        dict(name="float-group-is-not-admitted", rows={"a": row("missing_spot_price"),
                                                      "b": row("derived_transfer_group_blocked")},
             details={"a": {"transfer_group_id": 7.0}, "b": {"transfer_group_id": 7}}),
    ]
    for vector in vectors:
        vector["expected"] = quarantine_review.waiting_roots(vector["rows"], vector["details"])
    return vectors


def export():
    output = {"scenarios": [], "waiting": waiting_vectors()}
    variants = {
        "unprocessed": [],
        "count-stale": ["UPDATE profiles SET last_processed_tx_count=1 WHERE id='p'"],
        "version-stale": ["UPDATE profiles SET last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),last_processed_input_version=-1 WHERE id='p'"],
        "current": ["UPDATE profiles SET last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),last_processed_input_version=journal_input_version WHERE id='p'",
                    f"UPDATE profiles SET journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION} WHERE id='p'"],
        "empty": ["DELETE FROM transactions", "UPDATE profiles SET last_processed_tx_count=0 WHERE id='p'"],
        "receipts": [],
        "pair": [],
        "pair-note": [],
        "pair-allocation": [],
        "pair-same-txid": [],
        "sibling-pairs": [],
        "pair-large-exact": ["UPDATE transactions SET amount=9007199254740993,fee=0 WHERE id='a'", "UPDATE transactions SET amount=9007199254740991 WHERE id='b'"],
        "pair-reverse-time": [],
        "pair-other-asset": ["UPDATE transactions SET asset='LBTC' WHERE id='b'"],
    }
    for name, changes in variants.items():
        conn = template()
        conn.row_factory = sqlite3.Row
        seed(conn)
        conn.execute("DELETE FROM transactions WHERE id IN ('net','contradiction')")
        if name in ("pair", "pair-note", "pair-allocation"):
            conn.execute("UPDATE transactions SET fee=0 WHERE id='a'")
        if name == "unprocessed":
            conn.execute("UPDATE profiles SET last_processed_at=NULL WHERE id='p'")
        for statement in changes:
            conn.execute(statement)
        if name == "receipts":
            for index in range(12):
                receipt = dict(id=f"receipt-{index:02}", created_at=f"2025-01-{index+1:02}",
                               result_input_version=index, artifact_digest=f"{index:064x}",
                               status="verified", proposed_operations=["must stay private"])
                conn.execute("INSERT INTO review_workflow_receipts(id,workspace_id,profile_id,idempotency_key,artifact_digest,receipt_json,created_at) VALUES(?,'w','p',?,?,?,?)",
                             (receipt["id"], f"key-{index}", receipt["artifact_digest"], json.dumps(receipt), receipt["created_at"]))
        conn.commit()
        if name.startswith("pair") or name == "sibling-pairs":
            conn.execute("UPDATE transactions SET external_id=? WHERE id='a'", ("A"*64,))
            conn.execute("UPDATE transactions SET external_id=? WHERE id='b'", ("b"*64,))
            conn.commit()
            with ExitStack() as stack:
                identity_name = "pair" if name in ("pair-note", "pair-allocation") else name
                identities = iter(uuid.uuid5(uuid.NAMESPACE_DNS, f"review-case-{identity_name}-{i}") for i in range(100))
                stack.enter_context(patch.object(uuid, "uuid4", side_effect=lambda: next(identities)))
                for module, field in ((custody_review_terms, "now_iso"),
                                      (custody_authored_migration, "now_iso"),
                                      (custody_components, "_now_iso")):
                    stack.enter_context(patch.object(module, field, return_value=CLOCK))
                try:
                    pair = handlers.create_transaction_pair(conn, "w", "p", "a", "b")
                except Exception as error:
                    error.add_note(f"Synthetic book: {name}; details: {getattr(error, 'details', None)}")
                    raise
                if name == "sibling-pairs":
                    handlers.create_transaction_pair(conn, "w", "p", "a", "c")
                if name == "pair-note":
                    handlers.update_transaction_pair(conn, "w", "p", pair["id"], notes="Only the explanatory note changed")
                if name == "pair-allocation":
                    record = quarantine_review.pairs_by_id(conn, "p")[pair["id"]]
                    component = custody_components.get_component(conn, record["component_id"])
                    legs = [{key:value for key,value in leg.items()
                             if key not in ("component_id","ordinal","created_at")}
                            for leg in component["legs"]]
                    suspense = next(leg for leg in legs if leg["role"] == "suspense")
                    half = suspense["amount_msat"] // 2
                    suspense["amount_msat"] = half
                    legs.append({**suspense, "id":"paid-out", "role":"external", "amount_msat":half})
                    source = next(leg for leg in legs if leg["role"] == "source")
                    allocations = [{key:a[key] for key in ("source_leg_id","sink_leg_id","source_amount_msat","sink_amount_msat")}
                                   for a in component["allocations"]]
                    for allocation in allocations:
                        if allocation["sink_leg_id"] == suspense["id"]:
                            allocation["source_amount_msat"] = allocation["sink_amount_msat"] = half
                    allocations.append(dict(source_leg_id=source["id"], sink_leg_id="paid-out",
                                            source_amount_msat=half,sink_amount_msat=half))
                    version = conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
                    custody_component_planner.apply_component_review(
                        conn, workspace_id="w", profile_id="p", expected_input_version=version,
                        action="revise", component_id=component["id"], spec=dict(legs=legs,allocations=allocations),
                        activate=True, reason="Half of the residual paid a third party",
                    )
            if name == "pair-reverse-time":
                conn.execute("UPDATE transactions SET occurred_at='2024-01-01T00:00:00Z' WHERE id='b'")
                conn.commit()
            if name == "pair-same-txid":
                conn.execute("UPDATE transactions SET external_id=? WHERE id='b'", ("  "+"a"*64+"  ",))
                conn.commit()
        profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        # Open-time migration audits stamp SQLite's own clock, which the
        # patched now_iso() does not reach, and refuse updates; pin the
        # dumped stamp so the vectors are stable.
        statements = [re.sub(r"'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z'\);$", f"'{CLOCK}');", s)
                      if s.startswith('INSERT INTO "schema_migration_audits"') else s
                      for s in _iterdump(conn)]
        schema = [s for s in statements if s.startswith("CREATE TABLE")]
        objects = [s for s in statements if not s.startswith(("CREATE TABLE", "INSERT INTO", "BEGIN", "COMMIT"))]
        if "schema" not in output:
            output.update(schema=schema, objects=objects)
        assert output["schema"] == schema and output["objects"] == objects
        state, record = build(conn, profile)
        scope = [profile["id"], profile["journal_input_version"], profile["last_processed_at"]]
        payload = dict(scope=scope, after="")
        arguments = [{}, dict(limit=1), dict(limit=100)]
        arguments += [dict(limit=value) for value in (0, 101, True, 1.0, "1", None)]
        arguments += [dict(cursor=value) for value in ("", "bad", 1, {}, "\ud800", "🍀")]
        arguments += [dict(cursor=token(value)) for value in (
            {}, [], None, {"scope": scope}, {"scope": ["foreign", *scope[1:]], "after": None},
            {"scope": scope, "after": 1}, {"scope": [scope[0], scope[1]+1, scope[2]], "after": ""},
            {"scope": [*scope[:2], "processed-again"], "after": ""},
            {"scope": [scope[0], float(scope[1]), scope[2]], "after": ""},
            {"scope": scope, "after": "\ud800"}, payload,
        )]
        arguments += [dict(cursor=token(payload).rstrip("=")), dict(cursor=token(payload)+"!"),
                      dict(cursor="!"+token(payload)), dict(cursor=token(payload)+"=ignored")]
        arguments += [dict(cursor=token(payload)+"🍀"), dict(cursor="钱"+token(payload)),
                      dict(cursor=token(payload)[:8]+"🦀"+token(payload)[8:])]
        cases = []
        with patch.object(review, "_build", return_value=state) as builder:
            page = review.inspect_cases(conn, profile, limit=1)
            if page["next_cursor"]:
                arguments += [dict(limit=1, cursor=page["next_cursor"])]
            for args in arguments:
                builder.reset_mock()
                try:
                    result = review.inspect_cases(conn, profile, **args)
                except Exception as error:
                    expected = {"error": dict(code=getattr(error, "code", "internal_error"),
                                              message=str(error), hint=getattr(error, "hint", None),
                                              details=getattr(error, "details", None))}
                else:
                    expected = {"result": result}
                cases.append(dict(args=args, build_calls=builder.call_count, **expected))
        record.pop("snapshot")  # The real loader is checked again by the live verifier.
        pair_readings = []
        for pair in quarantine_review.pairs_by_id(conn, "p").values():
            for side in ("out", "in"):
                transaction_id = pair[f"{side}_transaction_id"]
                pair_readings.append(dict(pair=pair, transaction_id=transaction_id,
                                          expected=quarantine_review.pair_evidence(conn, transaction_id, pair)))
        output["scenarios"].append(dict(name=name, sql=[s for s in statements if s.startswith("INSERT INTO")],
                                        build=record, quarantines=state["quarantines"], cases=cases, pair_readings=pair_readings))
        conn.close()
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = export()
    encoded = json.dumps(result, ensure_ascii=True, separators=(",", ":")) + "\n"
    if args.check:
        assert DESTINATION.read_text() == encoded, "review case vectors differ from Python"
    else:
        DESTINATION.write_text(encoded)
    print(dict(books=len(result["scenarios"]), reads=sum(len(s["cases"]) for s in result["scenarios"]),
               waiting=len(result["waiting"])))
