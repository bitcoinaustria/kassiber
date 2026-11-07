#!/usr/bin/env python3
"""Freeze economic/policy effects from unchanged Python; never authorize a plan."""
import argparse
from unittest.mock import patch
from copy import deepcopy
from decimal import Decimal
import json
from pathlib import Path
import sqlite3
import sys
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import review_workflow as review
from kassiber.core.custody_quantity_runtime import CanonicalQuantityState
from tests.custody_snapshot import to_jsonable

DESTINATION = ROOT / "crates/transfers/tests/review_effects.json"
PROFILE = dict(id="p", workspace_id="w", label="private label", tax_country="at",
               tax_long_term_days=365, tax_future_policy="future value", gains_algorithm="FIFO",
               fiat_currency="EUR", cost_basis_pool_scope="global", require_coarse_review=1,
               bitcoin_rail_carrying_value=0, journal_input_version=17)
IDENTITY = "0123456789abcdef0123456789abcdef"
OPERATIONS = [dict(type="kind_override", transaction_id="tx", kind="income", reason="Reviewed")]


def decode(value):
    if isinstance(value, list):
        return [decode(item) for item in value]
    if not isinstance(value, dict):
        return value
    if "$decimal" in value:
        return Decimal(value["$decimal"])
    if "$pairs" in value:
        return {tuple(decode(key)): decode(item) for key, item in value["$pairs"]}
    if "$type" in value:
        fields = {key: decode(item) for key, item in value.items() if key != "$type"}
        return CanonicalQuantityState(**fields) if value["$type"] == "CanonicalQuantityState" else SimpleNamespace(**fields)
    return {key: decode(item) for key, item in value.items()}


def connection(profile, identity=IDENTITY):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("CREATE TABLE profiles(" + ",".join('"' + key + '"' for key in profile) + ")")
    conn.execute("INSERT INTO profiles VALUES(" + ",".join("?" for _ in profile) + ")", list(profile.values()))
    conn.execute("CREATE TABLE settings(key TEXT,value)")
    if identity is not None:
        conn.execute("INSERT INTO settings VALUES('database_instance_id',?)", (identity,))
    conn.commit()
    return conn


def expected(state, profile=PROFILE, identity=IDENTITY):
    with connection(profile, identity) as conn:
        try:
            economic = dict(economic=review._effects(state))
        except Exception as error:
            economic = dict(economic_error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details))
        try:
            full = dict(result=review._effects(state, conn, profile, OPERATIONS))
        except Exception as error:
            full = dict(error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details))
        return dict(**economic, **full)


def base():
    entries = []
    for index, kind in enumerate(("acquisition", "income", "disposal", "fee", "transfer_fee", "transfer_in", "transfer_out")):
        entries.append(dict(id=f"derived-{index}", workspace_id="w", profile_id="p", transaction_id=f"tx-{index}",
                            wallet_id="wallet 钱 🦀", account_id=None, occurred_at="2025-02-01T00:00:00Z",
                            entry_type=kind, asset="BTC" if index % 2 else "LBTC", quantity=Decimal("1.2300E-8"),
                            fiat_value=Decimal("9007199254740993.0001"), unit_cost=Decimal("-0.00"),
                            cost_basis=Decimal("2.500"), proceeds=Decimal("1E-7"), gain_loss=Decimal("-2.4999999"),
                            description="Non-economic description", at_category="capital_gains", at_kennzahl=994,
                            capital_gains_type="long_term"))
    return dict(entries=entries, quarantines=[], tax_summary=[], wallet_holdings={},
                custody_component_blockers=[], custody_quantity=CanonicalQuantityState(None, None, (), None))


def states():
    yield "entry-types-exact-scale", base()
    empty = base()
    empty["entries"] = []
    yield "empty", empty
    for field in ("transaction_id", "wallet_id", "account_id", "occurred_at"):
        for value in (None, True, 9007199254740993, 1.0, "钱 🦀"):
            state = base()
            state["entries"][0][field] = value
            yield f"scalar-{field}-{value!r}", state
    for field in ("cost_basis", "proceeds", "gain_loss", "at_category", "at_kennzahl", "capital_gains_type"):
        state = base()
        for entry in state["entries"]:
            entry.pop(field)
        yield f"absent-{field}", state
    for number in ("0", "-0.000", "1E-308", "1E+308", "1.000000000000000000000000000000009", "1.20"):
        state = base()
        for entry in state["entries"]:
            for field in ("fiat_value", "cost_basis", "proceeds", "gain_loss"):
                entry[field] = Decimal(number)
        yield f"decimal-{number}", state
    state = base()
    state["entries"][0]["id"] = "changed-derived-id"
    state["entries"][0]["description"] = "changed-non-economic-description"
    yield "ignore-derived-id-and-description", state
    for cause in ("component", "quantity", "both"):
        state = base()
        state["custody_component_blockers"] = [dict(code="blocked")] if cause != "quantity" else []
        state["custody_quantity"] = CanonicalQuantityState(None, None, ("issue",) if cause != "component" else (), None)
        yield f"block-{cause}", state
    for count in (1, 100, 101, 150):
        state = base()
        state["quarantines"] = [dict(transaction_id=f"tx-{i:03}", reason="missing_spot_price", detail_json="{}") for i in range(count)]
        state["quarantines"] += [dict(state["quarantines"][0]), dict(transaction_id="tx-000", reason="custody_quantity_unresolved")]
        state["wallet_holdings"] = {(f"wallet-{i:03}", "account", "p", "BTC"): dict(quantity=Decimal("1.000"), cost_basis=Decimal("2.00")) for i in range(count)}
        yield f"bounded-{count}", deepcopy(state)
        state["entries"].reverse()
        state["quarantines"].reverse()
        state["wallet_holdings"] = dict(reversed(list(state["wallet_holdings"].items())))
        yield f"reordered-{count}", deepcopy(state)
        if count > 100:
            state["wallet_holdings"][(f"wallet-{count-1:03}", "account", "p", "BTC")]["cost_basis"] = Decimal("3.00")
            state["quarantines"][0]["detail_json"] = "{\"ignored\":true}"
            yield f"hidden-holding-{count}", state
    for number in (-0.0, 1e-20, 1e20, 1.2345678901234567):
        state = base()
        state["tax_summary"] = [dict(year=2025, asset="BTC", transaction_type="disposal", capital_gains_type="short_term",
                                     quantity=number, quantity_msat=9007199254740993,
                                     proceeds=number, cost_basis=number, gain_loss=number)]
        yield f"tax-summary-{number!r}", state
    for field in ("quantity", "proceeds", "cost_basis", "gain_loss"):
        for number in (float("nan"), float("inf"), float("-inf")):
            state = base()
            row = dict(year=2025, asset="BTC", transaction_type="disposal", capital_gains_type="short_term",
                       quantity=0.0, quantity_msat=1, proceeds=0.0, cost_basis=0.0, gain_loss=0.0)
            row[field] = number
            state["tax_summary"] = [row]
            yield f"tax-summary-{field}-{number!r}", state


def export():
    result = dict(rendered=[], synthetic=[], live=[])
    for index, case in enumerate(json.loads((ROOT / "crates/journals/tests/data/review-builds.json").read_text())):
        if "state" not in case["expected"]:
            continue
        state = decode(json.loads(case["expected"]["state"]))
        profile = case["snapshot"]["profile"]
        result["rendered"].append(dict(index=index, profile=profile, **expected(state, profile)))
    fixture = json.loads((ROOT / "crates/transfers/tests/review_cases.json").read_text())
    for scenario in fixture["scenarios"]:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.executescript("\n".join([*fixture["schema"], *scenario["sql"], *fixture["objects"]]))
        state = decode(json.loads(scenario["build"]["expected"]["state"]))
        result["live"].append(dict(name=scenario["name"], economic=review._effects(state),
                                  result=review._effects(state, conn, scenario["build"]["profile"], OPERATIONS)))
        conn.close()
    for name, state in states():
        result["synthetic"].append(dict(name=name, state=to_jsonable(state), profile=PROFILE,
                                         identity=IDENTITY, **expected(state)))
    for field in PROFILE:
        profile = {**PROFILE, field: "changed" if isinstance(PROFILE[field], str) else 0}
        if field in ("id", "workspace_id"):
            continue
        result["synthetic"].append(dict(name=f"policy-{field}", state=to_jsonable(base()), profile=profile,
                                         identity=IDENTITY, **expected(base(), profile)))
    for identity in (None, "bad", "F" * 32, 42, "a" * 32):
        result["synthetic"].append(dict(name=f"identity-{identity!r}", state=to_jsonable(base()), profile=PROFILE,
                                         identity=identity, **expected(base(), identity=identity)))
    state = base()
    state["tax_summary"] = [dict(year=2025, asset="BTC", transaction_type="disposal", capital_gains_type="short_term",
                                 quantity=float("nan"), quantity_msat=1, proceeds=0.0, cost_basis=0.0, gain_loss=0.0)]
    result["synthetic"].append(dict(name="identity-before-finite-json", state=to_jsonable(state), profile=PROFILE,
                                     identity="bad", **expected(state, identity="bad")))
    return json.dumps(result, ensure_ascii=True, separators=(",", ":")) + "\n"


def without_tax(export):
    """Since #680 `_effects` adds per-year tax figures read from a stored
    journal, which these synthetic states and toy connections do not have.
    The figures are compared on real books by tests/test_rust_review_terms.py;
    here `tax` is stubbed out and dropped, so the rest of `_effects` stays pinned."""
    original = review._effects

    def effects(state, conn=None, profile=None, operations=()):
        # The economic part alone, as `_effects(state)` read before #680.
        result = original(state, conn, profile, operations)
        assert result.pop("tax") is None
        return result

    def run():
        with patch.object(review, "_tax_effects", lambda *args, **kwargs: None), \
                patch.object(review, "_effects", effects):
            return export()
    return run


if __name__ == "__main__":
    export = without_tax(export)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = export()
    if args.check:
        assert DESTINATION.read_text() == encoded, "review effect fixtures differ from Python"
    else:
        DESTINATION.write_text(encoded)
    print({key: len(value) for key, value in json.loads(encoded).items()})
