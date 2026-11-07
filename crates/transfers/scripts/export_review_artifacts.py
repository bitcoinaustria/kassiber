#!/usr/bin/env python3
"""Freeze portable construction and pre-transaction admission from Python.

Construction stubs the accounting work; admission stops before transaction
ownership is read. These vectors claim neither plan nor apply parity.
"""
import argparse
from contextlib import contextmanager, nullcontext
from copy import deepcopy
import json
from pathlib import Path
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import custody_components, journal_storage, review_workflow as review
from kassiber.errors import AppError

DESTINATION = ROOT / "crates/transfers/tests/review_artifacts.json"
PROFILE = dict(id="p", workspace_id="w")
# Since #680 effects carry per-year tax figures and the artifact their delta.
TAX = dict(fiat_currency="EUR", jurisdiction=None, years_omitted=0)
BEFORE = dict(journal_digest="before", report_ready=False,
              tax=dict(TAX, years=[dict(year=2024, realized_gain_loss="10.00", income="0.00")]))
AFTER = dict(journal_digest="after", report_ready=True,
             tax=dict(TAX, years=[dict(year=2024, realized_gain_loss="12.50", income="0.00")]))
BASE = dict(schema_version=1, workspace_id="w", profile_id="p", base_input_version=17,
            operations=[dict(type="exclude", transaction_id="tx", reason="Reviewed")],
            before=BEFORE, after=AFTER, delta=review._tax_delta(BEFORE, AFTER))


def signed(payload):
    return {**payload, "digest": review._digest(payload)}


class Admitted(Exception):
    pass


class StopBeforeTransaction:
    @property
    def in_transaction(self):
        raise Admitted


def admission(artifact, key):
    try:
        with patch.object(journal_storage, "journal_write_cache", return_value=nullcontext()):
            review.apply_review(StopBeforeTransaction(), PROFILE, artifact=artifact, idempotency_key=key, hooks=None)
    except Admitted:
        return dict(result=review._operations(artifact["operations"]))
    except AppError as error:
        return dict(error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details))
    except AttributeError as error:
        # A bare exception: the daemon's internal_error envelope.
        return dict(error=dict(code="internal_error", message=str(error), hint=None, details=None))
    raise AssertionError("apply crossed the pre-transaction boundary")


def assembly(payload):
    @contextmanager
    def snapshot(conn):
        yield conn
    try:
        with patch.object(review, "_snapshot", snapshot), patch.object(review, "_profile", return_value=PROFILE), \
             patch.object(custody_components, "require_review_input_version"), \
             patch.object(review, "_build"), patch.object(review, "_apply_operations"), \
             patch.object(review, "_effects", side_effect=[payload["before"], payload["after"]]):
            artifact = review.plan_review(None, PROFILE, operations=payload["operations"],
                                         expected_input_version=payload["base_input_version"], hooks=None)
            return dict(result=artifact)
    except AppError as error:
        return dict(error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details))


def candidates():
    artifact = signed(BASE)
    yield "canonical", artifact, "key"
    yield "mapping-order", dict(reversed(list(artifact.items()))), "key"
    for value in (None, False, 0, [], "artifact", {}, {**artifact, "extra": True}):
        yield f"shape-{value!r}"[:100], value, None
    for field in artifact:
        yield f"missing-{field}", {key: value for key, value in artifact.items() if key != field}, "key"
        for value in (None, True, 1.0, "", "other", [], {}):
            payload = {**BASE, field: value} if field != "digest" else BASE
            changed = signed(payload)
            if field == "digest":
                changed["digest"] = value
            yield f"{field}-{value!r}", changed, "key"
    for operations in (
        [dict(type="exclude", transaction_id=" tx ", reason="Reviewed")],
        [dict(type="price_override", transaction_id="tx", fiat_rate="1E3", reason="Reviewed")],
        [dict(type="price_override", transaction_id="tx", fiat_rate="1.000", reason="Reviewed")],
        [dict(type="kind_override", transaction_id="tx", kind=None, reason="Reviewed")],
        [dict(type="custody_component", request=dict(action="create", components=[dict(amount_msat=9007199254740993, note="钱 🦀")]))],
        list(reversed(BASE["operations"] + [dict(type="exclude", transaction_id="other", reason="Second")])),
    ):
        yield "operations-" + repr(operations)[:80], signed({**BASE, "operations": operations}), "key"
    for key in (None, False, 1, [], {}, "", "\x1c \u2003", " key ", "x" * 200, "x" * 201, " " * 300 + "x", "🦀" * 200, "\ud800"):
        yield f"idempotency-{key!r}"[:100], artifact, key
    # Refusal precedence, including scope before operation normalization.
    yield "digest-before-scope", {**artifact, "profile_id": "other"}, None
    yield "scope-before-operations", signed({**BASE, "profile_id": "other", "operations": None}), None
    yield "operations-before-key", signed({**BASE, "operations": None}), None
    yield "nonfinite-before-digest", {**artifact, "before": float("nan")}, None
    yield "nonfinite-malformed", {**artifact, "extra": float("nan")}, None
    # The delta is checked after the operations and before the key.
    yield "delta-mismatch", signed({**BASE, "delta": dict(years=[])}), None
    yield "delta-malformed", signed({**BASE, "before": dict(BEFORE, tax=None)}), None
    yield "delta-attribute", signed({**BASE, "before": dict(BEFORE, tax=dict(TAX, years=[
        dict(year=2024, realized_gain_loss="1", income="0", kennzahlen=[172])]))}), "key"
    yield "delta-before-key", signed({**BASE, "delta": None}), None
    yield "operations-before-delta", signed({**BASE, "operations": None, "delta": None}), "key"


def export():
    result = dict(assembly=[], admission=[])
    for name, artifact, key in candidates():
        result["admission"].append(dict(name=name, artifact=artifact, key=key, **admission(artifact, key)))
    for name, payload in (("base", BASE), ("unicode", {**BASE, "before": dict(note="钱 🦀", tax=BEFORE["tax"])}),
                          ("exact", {**BASE, "before": dict(amount=9007199254740993, number=-0.0, tax=BEFORE["tax"])}),
                          ("ordered", {**BASE, "operations": BASE["operations"] * 2}),
                          ("no-tax-change", {**BASE, "after": dict(AFTER, tax=BEFORE["tax"])})):
        result["assembly"].append(dict(name=name, payload=payload, **assembly(payload)))
    for character in ("a", "é", "🦀"):
        payload = deepcopy(BASE)
        payload["before"] = dict(padding="", tax=BEFORE["tax"])
        overhead = len(review._json(signed(payload)).encode())
        width = len(review._json(character).encode()) - 2
        for count in ((review.MAX_ARTIFACT_BYTES - overhead) // width,
                      (review.MAX_ARTIFACT_BYTES - overhead) // width + 1):
            payload["before"]["padding"] = character * count
            recipe = dict(character=character, count=count)
            actual = assembly(payload)
            if "result" in actual:
                actual = dict(encoded_bytes=len(review._json(actual["result"]).encode()), digest=actual["result"]["digest"])
            result["assembly"].append(dict(name=f"bound-{character}-{count}", recipe=recipe, **actual))
            result["admission"].append(dict(name=f"bound-{character}-{count}", recipe=recipe, key="key", **admission(signed(payload), "key")))
    return json.dumps(result, ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = export()
    if args.check:
        assert DESTINATION.read_text() == encoded, "review artifact fixtures differ from Python"
    else:
        DESTINATION.write_text(encoded)
    print({key: len(value) for key, value in json.loads(encoded).items()})
