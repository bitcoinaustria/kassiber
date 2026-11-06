#!/usr/bin/env python3
"""Freeze review-operation normalization and content digests from unchanged Python."""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import review_workflow as review
from kassiber.errors import AppError

DESTINATION = ROOT / "crates/transfers/tests/review_operations.json"


def inputs():
    base = dict(type="exclude", transaction_id="  tx 钱 🦀  ", reason=" \x1c reviewed\n ")
    yield from (None, {}, [], [None], [True], ["exclude"], [base] * 51, [base] * 50)
    for kind in (None, False, 1, [], {}, "unsupported", "exclude", "price_override", "kind_override", "unpair", "custody_component"):
        yield [{**base, "type": kind}]
    for field in ("type", "transaction_id", "reason"):
        for value in (None, False, 42, [], {}, "", " \u2003\x1f\n", "é" * 2000, "é" * 2001, "\ud800"):
            yield [{**base, field: value}]
        yield [{key: value for key, value in base.items() if key != field}]
    for price in (None, False, 42, [], {}, "", "no", "NaN", "sNaN", "Infinity", "-Infinity",
                  "0", "-0E-999", "-1", "1.2300", "1E+3", "1e-13", "9e-14", "1e-309", "1e309",
                  "1." + "0" * 308, "1." + "0" * 309, "1" * 512, "1" * 513,
                  "\x1c1.25\x1f", "１.２５", "1_2.5", "9007199254740993.0001", "\ud800"):
        for field in ("fiat_rate", "fiat_value"):
            yield [{**base, "type": "price_override", field: price}]
    yield [{**base, "type": "price_override", "fiat_rate": "1", "fiat_value": "2"}]
    yield [{**base, "extra": 1}]
    yield [{**base, "\ud800": 1}]
    yield [{**base, "extra": float("nan")}]
    for kind in (None, True, [], {}, "buy", "deposit", "income", "mining", "staking", "wages", "interest",
                 "lending_interest", "airdrop", "hardfork", "sell", "buy ", "hard_fork", "unsupported"):
        for mode in ("market_value", "cost", None):
            yield [{**base, "type": "kind_override", "kind": kind, "valuation_mode": mode}]
    unpair = dict(type="unpair", pair_id=" p ", expected_fingerprint=" hash ", reason=" reviewed ")
    yield [unpair]
    for field in ("pair_id", "expected_fingerprint", "reason"):
        for value in (None, False, "", "\x1f ", "\ud800", "x" * 2001):
            yield [{**unpair, field: value}]
        yield [{key: value for key, value in unpair.items() if key != field}]
    for action in (None, [], {}, False, "unsupported", "create", "revise", "activate", "supersede", "undo"):
        yield [dict(type="custody_component", request=dict(action=action))]
    for request in (None, [], "", dict(action="create", extra=True), dict(action="create", components=[]),
                    dict(action="create", components=[dict(amount_msat=9007199254740993, note="钱 🦀")], activate=True),
                    dict(action="revise", component_id="component", spec=dict(notes="reviewed"), reason="reason"),
                    dict(action="supersede", component_id="component", reason="reason")):
        yield [dict(type="custody_component", request=request)]
    yield [dict(type="custody_component", request=dict(action="create"), extra=True)]
    yield [base, unpair, {**base, "type": "price_override", "fiat_value": "0"},
           {**base, "type": "kind_override", "kind": "income"}]
    yield from term_inputs()


def term_inputs():
    """#680's term operations and #708's coarse-price acceptance."""
    legs = dict(out_transaction_id=" out ", in_transaction_id=" in\n", reason=" reviewed ")
    pair = dict(type="pair", **legs)
    yield [pair]
    yield [{**pair, "pair_kind": " coinjoin ", "policy": "taxable", "out_amount_msat": 1}]
    for field in ("out_transaction_id", "in_transaction_id", "reason", "pair_kind", "policy"):
        for value in (None, False, 7, [], "", " \x1f ", "\ud800", "x" * 2000, "x" * 2001):
            yield [{**pair, field: value}]
    for field in ("out_transaction_id", "in_transaction_id", "reason"):
        yield [{key: value for key, value in pair.items() if key != field}]
    yield [{**pair, "extra": 1}]
    yield [{**pair, "expires_in_days": 5}]
    yield [{**pair, "in_transaction_id": "out"}]
    yield [{**pair, "in_transaction_id": "out", "out_transaction_id": " out\t"}]
    for kind in ("manual", "whirlpool", "chain-swap", "swap-refund", "Manual", "direct-swap-payout"):
        yield [{**pair, "pair_kind": kind}]
    for policy in ("carrying-value", "taxable", "carrying_value", "Taxable"):
        yield [{**pair, "policy": policy}]
    for amount in (0, -1, 1, 10 ** 30, True, False, 1.0, "1", None, [1]):
        yield [{**pair, "out_amount_msat": amount}]
    dismiss = dict(type="dismiss_pair", **legs)
    yield [dismiss]
    for days in (0, 1, 90, 3650, 3651, -1, True, 1.0, "5", None, 10 ** 30):
        yield [{**dismiss, "expires_in_days": days}]
    yield [{**dismiss, "pair_kind": "manual"}]
    yield [{**dismiss, "in_transaction_id": " out "}]
    undismiss = dict(type="undismiss_pair", **legs)
    yield [undismiss]
    yield [{**undismiss, "expires_in_days": 5}]
    yield [{key: value for key, value in undismiss.items() if key != "reason"}]
    gap = dict(type="custody_gap", action=" create ", gap_id=" gap:1 ", reason=" bridge ")
    yield [gap]
    for action in (None, 1, "", "unsupported", "Create", *review.CUSTODY_GAP_ACTIONS):
        yield [{**gap, "action": action}]
    for action in ("classify_residual", "create"):
        yield [{**gap, "action": action, "classification": " fee "}]
        yield [{**gap, "action": action, "classification": ""}]
        yield [{**gap, "action": action, "classification": None}]
    for reason in ("x" * 500, "x" * 501, " " + "x" * 500 + " ", "x" * 2001):
        yield [{**gap, "reason": reason}]
    yield [{**gap, "gap_id": "g" * 2001}]
    yield [{key: value for key, value in gap.items() if key != "gap_id"}]
    accept = dict(type="accept_coarse_prices", reason=" accepted ")
    yield [accept]
    for ids in ([" b ", "a", " c"], ["a"], [], ["a", " a "], ["a", "a"], ["", "a"], [" "], [None],
                [1], "a", None, {"a": 1}, [f"t{index}" for index in range(5000)],
                [f"t{index}" for index in range(5001)], ["\ud800", "a"]):
        yield [{**accept, "transaction_ids": ids}]
    for reason in (None, "", " ", 7, "x" * 2000, "x" * 2001):
        yield [{**accept, "reason": reason}]
    yield [{key: value for key, value in accept.items() if key != "reason"}]
    yield [{**accept, "transaction_id": "a"}]
    yield [pair, dismiss, undismiss, gap, accept, {**base_operation(), "type": "exclude"}]


def base_operation():
    return dict(type="exclude", transaction_id="tx", reason="reviewed")


def export():
    cases = []
    for operations in inputs():
        case = dict(operations=operations)
        try:
            result = review._operations(operations)
            case.update(canonical=review._json(result), digest=review._digest(result))
        except AppError as error:
            case["error"] = dict(code=error.code, message=str(error), hint=error.hint, details=error.details)
        except TypeError as error:
            # A bare exception reaches the daemon's internal_error envelope
            # (an unhashable operation type since #680).
            case["error"] = dict(code="internal_error", message=str(error), hint=None, details=None)
        cases.append(case)
    for character in ("a", "é", "🦀"):
        operation = dict(type="custody_component", request=dict(action="create", spec=dict(notes="")))
        overhead = len(review._json([operation]).encode())
        width = len(review._json(character).encode()) - 2
        for count in ((review.MAX_ARTIFACT_BYTES - overhead) // width,
                      (review.MAX_ARTIFACT_BYTES - overhead) // width + 1):
            operation["request"]["spec"]["notes"] = character * count
            case = dict(recipe=dict(character=character, count=count))
            try:
                result = review._operations([operation])
                case.update(encoded_bytes=len(review._json(result).encode()), digest=review._digest(result))
            except AppError as error:
                case["error"] = dict(code=error.code, message=str(error), hint=error.hint, details=error.details)
            cases.append(case)
    return json.dumps(cases, ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = export()
    if args.check:
        if DESTINATION.read_text() != encoded:
            raise SystemExit("review operation fixtures differ from current Python")
    else:
        DESTINATION.write_text(encoded)
    print(f"{len(json.loads(encoded))} review operation cases")
