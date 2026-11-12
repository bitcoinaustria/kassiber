"""Synthetic CPython component-stage baselines for Rust's named mutation tests."""
from __future__ import annotations
import json
from pathlib import Path
import sys

from kassiber.core.custody_compute import compute_components_stage
from tests.custody_snapshot import to_jsonable
from tests.custody_snapshot_generator import COMPONENT_SCENARIOS, component_snapshot

PATH = Path(__file__).parent / "fixtures/custody_component_cases.json"


def vectors():
    result = []
    for case in COMPONENT_SCENARIOS:
        book = component_snapshot(0, case=case)
        components, blockers = compute_components_stage(book)
        result.append({"case": case, "input": book, "components": to_jsonable({"components": components, "blockers": blockers}, redact_config=False)})
    for case, mutate in (
        ("refusal_mode", lambda c: c["header"].update(conservation_mode="future")),
        ("refusal_mode_token", lambda c: c["header"].update(conservation_mode="QUANTITY")),
        ("refusal_mode_missing", lambda c: c["header"].update(conservation_mode=None)),
        ("refusal_integer", lambda c: c["legs"][0].update(amount_msat="1__0")),
        ("refusal_null_integer", lambda c: c["legs"][0].update(amount_msat=None)),
        ("refusal_list_integer", lambda c: c["legs"][0].update(amount_msat=[])),
        ("refusal_dict_integer", lambda c: c["legs"][0].update(amount_msat={})),
        ("refusal_utc_underflow", lambda c: c["legs"][0].update(occurred_at="0001-01-01T00:00:00+01:00")),
        ("refusal_utc_overflow", lambda c: c["legs"][0].update(occurred_at="9999-12-31T23:59:59-01:00")),
        ("refusal_chronology_overflow", lambda c: c["legs"][1].update(occurred_at="9999-12-31T00:00:00Z")),
        ("refusal_missing_child_field", lambda c: c["legs"][0].pop("asset")),
        ("refusal_child_before_mode", lambda c: (c["legs"][0].pop("asset"), c["header"].update(conservation_mode="future"))),
        ("refusal_time_before_count", lambda c: (c["legs"][0].update(occurred_at="0001-01-01T00:00:00+01:00"), c["header"].update(expected_leg_count="invalid"))),
    ):
        book = component_snapshot(0)
        mutate(book["components"][0])
        try:
            compute_components_stage(book)
        except Exception as exc:
            result.append({"case": case, "input": book, "error_code": getattr(exc, "code", type(exc).__name__)})
        else:
            raise AssertionError(f"{case} must refuse")
    from copy import deepcopy
    def revision(book):
        c = book["components"][0]
        target = {**c["header"], "id": "previous", "revision": "abc", "state": "superseded"}
        c["header"]["supersedes_component_id"] = target["id"]
        book["revision_headers"].append(target)
    def competing(book):
        c = book["components"][0]
        book["revision_headers"].append({**c["header"], "id": "competing", "revision": "abc"})
    def excluded(book, field):
        book["anchors"]["out"].update(excluded=1, **{field: "abc"})
        book["transactions"] = [r for r in book["transactions"] if r["id"] != "out"]
    def order(book, kind):
        c = book["components"][0]
        other = deepcopy(c)
        other["header"].update(id="aaa", lineage_id="aaa")
        for leg in other["legs"]: leg["component_id"] = "aaa"
        c["header"]["created_at"] = "2025"
        if kind == "components":
            c["legs"][0]["amount_msat"] = "abc"
            other["legs"][0]["amount_msat"] = None
        else:
            c["allocations"] = [{"id": "a", "component_id": c["header"]["id"], "ordinal": 0, "source_amount_msat": "abc"}]
            other["legs"][0]["amount_msat"] = None
        book["components"].append(other)
    for case, mutate in (
        ("refusal_revision_target", revision),
        ("refusal_competing_revision", competing),
        ("refusal_excluded_amount", lambda b: excluded(b, "amount")),
        ("refusal_excluded_fee", lambda b: excluded(b, "fee")),
        ("refusal_global_leg_order", lambda b: order(b, "components")),
        ("refusal_legs_before_allocations", lambda b: order(b, "children")),
        ("refusal_evidence_count_before_ordinal", lambda b: (b["components"][0]["header"].update(expected_evidence_count="abc"), b["components"][0]["commitments"][0].update(ordinal=None))),
        ("commitment_missing_ignores_ordinal", lambda b: (b["components"][0]["header"].update(expected_evidence_count=None), b["components"][0]["commitments"][0].update(ordinal="abc"))),
    ):
        book = component_snapshot(0)
        mutate(book)
        try:
            components, blockers = compute_components_stage(book)
        except Exception as exc:
            result.append({"case": case, "input": book, "error_code": getattr(exc, "code", type(exc).__name__)})
        else:
            assert case == "commitment_missing_ignores_ordinal", case
            result.append({"case": case, "input": book, "components": to_jsonable({"components": components, "blockers": blockers}, redact_config=False)})
    def legacy_pair_fee(book, *, notes="reviewed_residual:network_fee", fee=1, fee_allocated=1, partial=False):
        # A legacy pair's fee leg whose source stops short of the full debit (8f).
        c = book["components"][0]
        book["anchors"]["out"].update(fee=fee)
        fee_leg = c["legs"][3]
        fee_leg["notes"] = notes
        fee_leg["amount_msat"] = fee_allocated
        base = {"component_id": c["header"]["id"], "workspace_id": c["header"]["workspace_id"],
                "profile_id": c["header"]["profile_id"], "created_at": c["header"]["created_at"]}
        c["allocations"] = [
            {**base, "id": f"{c['header']['id']}:allocation:{i}", "ordinal": i,
             "source_leg_id": c["legs"][0]["id"], "sink_leg_id": c["legs"][sink]["id"],
             "source_amount_msat": amount, "sink_amount_msat": amount}
            for i, (sink, amount) in enumerate(((1, 61 - fee_allocated), (2, 39), (3, fee_allocated)))
        ]
        c["header"]["expected_allocation_count"] = 3
        if partial:
            # A reviewed pair term makes the source a partial review, whose
            # coverage counts at most the observed fee it allocates (ae0ddfca0).
            c["economic_terms"] = [{
                "payout_amount_msat": None, "reviewed_source_amount_msat": 100,
                "source_leg_id": c["legs"][0]["id"], "swap_fee_msat": 0,
                "target_leg_id": c["legs"][1]["id"],
            }]
    for case, mutate in (
        ("legacy_pair_fee_hold", legacy_pair_fee),
        ("legacy_pair_fee_other_note", lambda b: legacy_pair_fee(b, notes="reviewed_residual:other")),
        ("legacy_pair_fee_no_observed_fee", lambda b: legacy_pair_fee(b, fee=0)),
        ("legacy_pair_fee_partial_overallocated", lambda b: legacy_pair_fee(b, fee_allocated=6, partial=True)),
    ):
        book = component_snapshot(0)
        mutate(book)
        components, blockers = compute_components_stage(book)
        result.append({"case": case, "input": book, "components": to_jsonable({"components": components, "blockers": blockers}, redact_config=False)})
    from kassiber.core.custody_journal import rates_from_transaction_rows
    for case, rows in (
        ("refusal_rate_exact", [{"asset":"BTC", "fiat_rate_exact":"abc", "fiat_value_exact":None, "fiat_rate":None, "fiat_value":None, "amount":1}]),
        ("refusal_rate_value", [{"asset":"BTC", "fiat_rate_exact":"1", "fiat_value_exact":"abc", "fiat_rate":None, "fiat_value":None, "amount":1}]),
        ("refusal_rate_legacy", [{"asset":"BTC", "fiat_rate":{}, "fiat_value":None, "amount":1}]),
        ("rate_precedence_ignores_later_junk", [{"asset":"BTC", "fiat_rate_exact":"1", "fiat_value_exact":None, "fiat_rate":None, "fiat_value":None, "amount":1}, {"asset":"BTC", "fiat_rate_exact":"abc"}]),
    ):
        book = component_snapshot(0)
        book["rate_rows"] = rows
        book["components"][0]["legs"][0]["amount_msat"] = "abc" if case.startswith("refusal_") else 100
        try:
            rates_from_transaction_rows(rows)
        except Exception as exc:
            result.append({"case":case, "input":book, "error_code":getattr(exc, "code", type(exc).__name__)})
        else:
            components, blockers = compute_components_stage(book)
            result.append({"case":case, "input":book, "components":to_jsonable({"components":components,"blockers":blockers},redact_config=False)})
    return result




def semantics():
    from kassiber.time_utils import parse_iso_datetime_or_none
    from datetime import datetime, timezone
    from itertools import product
    dates = ["2026-01-01", "20260101", "2026-W01-4", "2026W014", "2026-W01", "2026W01", "2020-W53-7", "2021-W53-1", "0001-01-01", "9999-12-31", "0000-01-01", "2024-02-29", "2023-02-29", "2026-13-01", "2026-01-32"]
    times = ["", "T00", "T01:02", "T01:02:03", "T010203", "T01.25", "T01:02,125", "T01:02:03.1234567", " 01:02:03Z", "t01:02:03", "🐍01:02:03", "T24:00:00", "T01:60:00", "T01:02:60", "T1:02:03", "T01:0203", "T0102:03", "T01:", "T01:02:03.", "T01::", "T01:.", "T01:02:", "T01:02::", "T01:02:.", "T01.", "T01..", "T010203:", "T01:02:03:", "T01:02:03,:", "T01:02:03.0:"]
    zones = [" Z", " +01:00", "\t-01:00", "", "Z", "+01", "-01:02", "+010203", "+01:02:03.1234567", "+00:00:00.5", "+00:90", "+00:00:90", "+23:59:59", "+24:00", "z", "+00:00Z"]
    epoch = datetime(1, 1, 1, tzinfo=timezone.utc)
    values = []
    separators = ["T", " ", "t", "🐍", "\t"]
    for date, time, separator, zone in product(dates, times, separators, zones):
        value = date + (separator + time[1:] if time else "") + zone
        try:
            parsed = parse_iso_datetime_or_none(value)
        except OverflowError:
            values.append({"input": value, "error_code": "OverflowError"})
            continue
        us = None
        if parsed is not None:
            delta = parsed - epoch
            us = (delta.days * 86400 + delta.seconds) * 1000000 + delta.microseconds
        values.append({"input": value, "us": us})
    # Deterministic differential fuzz remains separate from the Cartesian
    # grammar corpus and is regenerated by the checked-in-vector test.
    import random
    rng = random.Random(0xC0570D)
    for _ in range(9477):
        value = rng.choice(dates) + rng.choice(separators + ["", "+", "-"]) + rng.choice(times)[1:] + rng.choice(["", " ", "x", "xx", ":", "\\t"]) + rng.choice(zones)
        try:
            parsed = parse_iso_datetime_or_none(value)
            delta = None if parsed is None else parsed - epoch
            values.append({"input": value, "us": None if delta is None else (delta.days * 86400 + delta.seconds) * 1000000 + delta.microseconds})
        except OverflowError:
            values.append({"input": value, "error_code": "OverflowError"})
    integers = []
    for value in (None, True, False, 0, 1, -1, 1.9, -1.9, "0", "+١_٢_٣", "\u2003-９９\u0085", "12_", "_12", "1__2", "+_12", "12.0", "\x1c12", "12\x1f", "0x10", "²", "", " ", "1 2", [], {}):
        try:
            number = int(value)
        except (TypeError, ValueError):
            number = None
        integers.append({"input": value, "integer": number})
    uppercase = [{"input": chr(i), "upper": chr(i).upper()} for i in range(0x110000) if not 0xD800 <= i <= 0xDFFF and (chr(i).upper() != chr(i) or chr(i).lower() != chr(i))]
    # Unicode additions whose Rust casing differs from CPython 3.11.
    uppercase.extend({"input": chr(i), "upper": chr(i).upper()} for i in (
        0x19B, 0x264, 0x1C8A, 0xA7CD, 0xA7CF, 0xA7D3, 0xA7D5, 0xA7DB,
        *range(0x10D70, 0x10D86), *range(0x16EBB, 0x16ED4),
    ))
    return {"times": values, "integers": integers, "uppercase": uppercase}


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv:
        PATH.write_text(text)
        PATH.with_name("custody_component_semantics.json").write_text(json.dumps(semantics(), indent=2, ensure_ascii=True) + "\n")
    else:
        assert PATH.read_text() == text
        assert json.loads(PATH.with_name("custody_component_semantics.json").read_text()) == semantics()
