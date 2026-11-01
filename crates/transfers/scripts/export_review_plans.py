"""Freeze complete plans and recomputing validation from unchanged Python."""
import argparse
from copy import deepcopy
from contextlib import ExitStack
from dataclasses import replace
import json
from pathlib import Path
import sqlite3
import sys
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli import handlers
from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import custody_authored_migration, custody_components, custody_journal, custody_review_terms, quarantine_review, review_workflow as review

CLOCK = "2026-10-31T12:00:00Z"
DESTINATION = ROOT / "crates/transfers/tests/review_plans.json"


def outcome(call):
    try:
        return dict(result=call())
    except Exception as error:
        return dict(error=dict(code=getattr(error, "code", "internal_error"),
                               message=str(error), hint=getattr(error, "hint", None),
                               details=getattr(error, "details", None)))


def export():
    fixture = json.loads((ROOT / "crates/transfers/tests/review_cases.json").read_text())
    scenarios = {row["name"]: row for row in fixture["scenarios"]}
    output = []
    hooks = review.ReviewHooks(metadata=replace(_metadata_hooks(), now_iso=lambda: CLOCK))
    siblings = sqlite3.connect(":memory:")
    siblings.row_factory = sqlite3.Row
    siblings.executescript("\n".join([*fixture["schema"], *scenarios["pair"]["sql"], *fixture["objects"]]))
    siblings.execute("UPDATE transactions SET amount=200 WHERE id='c'")
    siblings.commit()
    with ExitStack() as stack:
        stack.enter_context(patch.object(uuid, "uuid4", return_value=uuid.UUID("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")))
        for module, field in ((custody_review_terms, "now_iso"), (custody_authored_migration, "now_iso"), (custody_components, "_now_iso")):
            stack.enter_context(patch.object(module, field, return_value=CLOCK))
        handlers.create_transaction_pair(siblings, "w", "p", "a", "c")
    scenarios["siblings-suspense"] = dict(sql=[statement for statement in siblings.iterdump() if statement.startswith("INSERT INTO")])
    siblings.close()

    def record(name, base, operations, setup=(), version=None, group="metadata"):
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.executescript("\n".join(statement.rstrip(";") + ";" for statement in [*fixture["schema"], *scenarios[base]["sql"], *fixture["objects"], *setup]))
        conn.execute("PRAGMA foreign_keys=ON")
        conn.commit()
        profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        if callable(operations):
            operations = operations(conn)
        if version is None:
            version = profile["journal_input_version"]
        build = lambda c, p: custody_journal.CustodyJournalBuilder(c, p, as_of=CLOCK).build()
        with patch.object(custody_journal, "build_ledger_state", side_effect=build):
            before = conn.total_changes
            expected = outcome(lambda: review.plan_review(conn, profile, operations=operations,
                               expected_input_version=version, hooks=hooks))
            assert conn.total_changes == before and not conn.in_transaction
            validations = []
            if "result" in expected:
                artifact = expected["result"]
                for change in (None, "digest", "before", "extra", "scope", "operations", "version"):
                    altered = deepcopy(artifact)
                    if change == "digest": altered["digest"] = "0" * 64
                    if change == "before": altered["before"]["report_ready"] = not altered["before"]["report_ready"]
                    if change == "extra": altered["extra"] = True
                    if change == "scope": altered["profile_id"] = "other"
                    if change == "operations": altered["operations"][0]["reason"] = "Changed review"
                    if change == "version": altered["base_input_version"] = version + 1
                    validations.append(dict(artifact=altered, **outcome(lambda: review.validate_review(conn, profile, artifact=altered, hooks=hooks))))
                for change, sql in (("policy", "UPDATE profiles SET tax_country='generic' WHERE id='p'"),
                                    ("evidence", "UPDATE transactions SET amount=amount+1 WHERE id='a'")):
                    conn.execute(sql)
                    conn.commit()
                    validations.append(dict(setup=sql, artifact=artifact, **outcome(lambda: review.validate_review(conn, profile, artifact=artifact, hooks=hooks))))
                    conn.close()
                    conn = sqlite3.connect(":memory:")
                    conn.row_factory = sqlite3.Row
                    conn.executescript("\n".join(statement.rstrip(";") + ";" for statement in [*fixture["schema"], *scenarios[base]["sql"], *fixture["objects"], *setup]))
                    conn.commit()
        conn.close()
        output.append(dict(name=name, group=group, base=base, setup=list(setup), operations=operations,
                           version=version, **expected, validations=validations,
                           **(dict(base_sql=scenarios[base]["sql"]) if base == "siblings-suspense" else {})))

    def price(id="a", **kwargs):
        return dict(type="price_override", transaction_id=id, fiat_rate="20000.00", reason="Invoice reviewed", **kwargs)

    def exclude(id="b"):
        return dict(type="exclude", transaction_id=id, reason="Duplicate reviewed")

    def kind(id="a", value="income"):
        return dict(type="kind_override", transaction_id=id, kind=value, reason="Acquisition reviewed")

    basic = ["DELETE FROM transactions", "UPDATE profiles SET journal_input_version=0 WHERE id='p'"]
    for id in ("a", "b"):
        basic.append(f"INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,raw_json,created_at) VALUES('{id}','w','p','source','{id}','2020-01-01T00:00:00Z','inbound','BTC',100000000000,0,'buy','{{}}','2020')")
    for name, operations in (
        ("mixed-metadata", [price(), exclude()]),
        ("reverse-metadata", [exclude(), price()]),
        ("repeat-price", [price(), price()]),
        ("price-after-exclusion", [exclude("a"), price()]),
        ("missing-target", [price("absent")]),
        ("kind-and-price", [kind(), price()]),
        ("clear-kind", [kind(value=None), price()]),
        ("value-only", [{**price(), "fiat_value": "0.12345678901234567890123456789012"}]),
        ("zero-value", [{k:v for k,v in {**price(), "fiat_value":"0"}.items() if k != "fiat_rate"}]),
    ):
        if name == "value-only": operations[0].pop("fiat_rate")
        record(name, "empty", operations, basic)
    record("stale-version", "empty", [price()], basic, version=1)
    record("invalid-version", "empty", [price()], basic, version=True)
    record("not-quarantined", "empty", [exclude("a")], [*basic, "UPDATE transactions SET fiat_rate=20000,fiat_rate_exact='20000' WHERE id='a'"])
    record("zero-quantity-price", "empty", [price()], [*basic, "UPDATE transactions SET amount=0 WHERE id='a'"])
    record("surrogate-transaction-id", "empty", [price("a\ud800")], basic)
    record("surrogate-run-transaction-id", "empty", [price("a\ud800\ud801")], basic)
    record("surrogate-operation-type", "empty", [{**price(), "type":"price_override\ud800"}], basic)
    for name, setup in (
        ("fractional-amount", "UPDATE transactions SET amount=1.5 WHERE id='a'"),
        ("bytes-amount", "UPDATE transactions SET amount=X'3737' WHERE id='a'"),
        ("bytes-fee", "UPDATE transactions SET fee=X'30' WHERE id='a'"),
        ("bytes-raw-json", "UPDATE transactions SET raw_json=X'7b7d' WHERE id='a'"),
        ("bytes-note", "UPDATE transactions SET note=X'ff00' WHERE id='a'"),
        ("bytes-label", "UPDATE wallets SET label=X'3737' WHERE id='source'"),
        ("bytes-currency", "UPDATE transactions SET fiat_currency=X'455552' WHERE id='a'"),
        ("bytes-profile-currency", "UPDATE profiles SET fiat_currency=X'455552' WHERE id='p'"),
        ("text-fee", "UPDATE transactions SET fee='unavailable' WHERE id='a'"),
        ("negative-amount", "UPDATE transactions SET amount=-1 WHERE id='a'"),
    ):
        record("raw-" + name, "empty", [price()], [*basic, setup], group="raw")
        if name in {"fractional-amount", "bytes-amount", "negative-amount"}:
            record("raw-value-" + name, "empty", [dict(type="price_override", transaction_id="a", fiat_value="20000.00", reason="Invoice reviewed")], [*basic, setup], group="raw")
    for name, reason in (
        ("surrogate", "Reviewed \ud800"),
        ("surrogate-run", "Reviewed \ud800\ud801"),
        ("astral-surrogate", "🚀 \ud800"),
        ("long", "x" * 1001),
        ("blank", " \u2003 "),
    ):
        record("author-" + name, "empty", [{**kind(), "reason": reason}], basic, group="authoring")
        record("noop-author-" + name, "empty", [{**kind(value=None), "reason": reason}], basic, group="authoring")
    for name, value in (("surrogate", "income\ud800"), ("surrogate-run", "income\ud800\ud801"),
                        ("long-surrogate", "x" * 64 + "\ud800")):
        record("kind-author-" + name, "empty", [kind(value=value)], basic, group="authoring")
    component_setup = [*basic,
        "UPDATE transactions SET direction='outbound',kind='send',occurred_at='2021-01-01T00:00:00Z',fiat_rate=30000,fiat_rate_exact='30000',raw_json='{\"privacy_boundary\":\"coinjoin\"}' WHERE id='b'",
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,fiat_rate,fiat_rate_exact,raw_json,created_at) VALUES('c','w','p','target','c','2022-01-01T00:00:00Z','inbound','BTC',100000000000,0,'receive',40000,'40000','{\"privacy_boundary\":\"coinjoin\"}','2020')"]
    component = dict(type="custody_component", request=dict(action="create", activate=True, components=[dict(
        component_type="manual_bridge", evidence_kind="manual_claim", evidence_grade="reviewed",
        change_reason="Both custody endpoints reviewed", legs=[dict(role="source", transaction="b", amount_msat=100000000000),
        dict(role="destination", transaction="c", amount_msat=100000000000)])]))
    for field in ("change_reason", "evidence_kind", "evidence_ref"):
        for suffix in ("\ud800", "\ud800\ud801"):
            authored = deepcopy(component)
            authored["request"]["components"][0][field] = "🚀 reviewed " + suffix
            record("component-author-" + field + str(len(suffix)), "empty", [authored], component_setup, group="authoring")
    for suffix in ("\ud800", "\ud800\ud801"):
        authored = deepcopy(component)
        authored["request"]["components"][0]["legs"][1] = dict(role="destination", untracked_wallet="🚀 missing " + suffix,
            occurred_at="2022-01-01T00:00:00Z", amount_msat=100000000000)
        record("component-author-wallet" + str(len(suffix)), "empty", [authored], component_setup, group="authoring")
    record("price-component", "empty", [price(), component], component_setup, group="components")
    record("component-price", "empty", [component, price()], component_setup, group="components")
    record("component-guards-acquisition", "empty", [component, kind("c")], component_setup, group="components")
    record("outbound-acquisition", "empty", [kind("b")], component_setup, group="components")

    def unpairs(conn):
        return [dict(type="unpair", pair_id=id, expected_fingerprint=quarantine_review.pair_fingerprint(conn, pair), reason="Owner reviewed custody")
                for id, pair in quarantine_review.pairs_by_id(conn, "p").items()]

    for suffix in ("\ud800", "\ud800\ud801"):
        record("revision-author-reason" + str(len(suffix)), "pair", lambda conn, suffix=suffix: [dict(
            type="custody_component", request=dict(action="revise", component_id=next(iter(
                quarantine_review.pairs_by_id(conn, "p").values()))["component_id"],
                spec={}, activate=False, reason="🚀 reviewed " + suffix))], group="authoring")
        record("unpair-unused-reason" + str(len(suffix)), "pair", lambda conn, suffix=suffix: [
            {**operation, "reason": "🚀 reviewed " + suffix} for operation in unpairs(conn)], group="authoring")

    for base in ("pair", "pair-note", "pair-allocation", "pair-same-txid", "sibling-pairs"):
        record(base, base, unpairs, group="unpairs")
    record("sibling-reversed", "sibling-pairs", lambda c: unpairs(c)[::-1], group="unpairs")
    record("siblings-with-suspense", "siblings-suspense", unpairs, group="unpairs")
    record("siblings-with-suspense-reversed", "siblings-suspense", lambda c: unpairs(c)[::-1], group="unpairs")
    record("duplicate-unpair", "pair", lambda c: unpairs(c)*2, group="unpairs")
    record("unknown-unpair", "pair", [dict(type="unpair", pair_id="absent", expected_fingerprint="x", reason="Reviewed")], group="unpairs")
    record("surrogate-pair-id", "pair", lambda c: [{**op, "pair_id": op["pair_id"]+"\ud800"} for op in unpairs(c)], group="unpairs")
    record("surrogate-pair-fingerprint", "pair", lambda c: [{**op, "expected_fingerprint": op["expected_fingerprint"]+"\ud800"} for op in unpairs(c)], group="unpairs")
    record("precheck-before-metadata", "sibling-pairs", lambda c: [price("absent"), {**unpairs(c)[1], "expected_fingerprint":"stale"}], group="unpairs")
    record("refresh-after-metadata", "pair", lambda c: [price("b"), *unpairs(c)], group="refresh")
    record("refresh-after-sibling-metadata", "sibling-pairs", lambda c: [unpairs(c)[0], price("d"), unpairs(c)[1]], group="refresh")
    record("refresh-after-suspense-sibling-metadata", "siblings-suspense", lambda c: [unpairs(c)[0], price("d"), unpairs(c)[1]], group="refresh")
    record("refresh-after-supersede", "pair", lambda c: [dict(type="custody_component", request=dict(action="supersede", component_id=next(iter(quarantine_review.pairs_by_id(c,"p").values()))["component_id"], reason="Reopened evidence")), *unpairs(c)], group="refresh")
    return json.dumps(output, ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = export()
    if args.check:
        assert DESTINATION.read_text() == encoded, "Review plan fixtures differ from Python"
    else:
        DESTINATION.write_text(encoded)
    print(f"{len(json.loads(encoded))} complete Python planning vectors")
