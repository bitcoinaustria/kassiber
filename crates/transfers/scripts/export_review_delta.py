#!/usr/bin/env python3
"""Freeze `review_workflow._tax_delta` over planner-shaped and untrusted effects.

Apply recomputes the delta from an artifact's own `before` and `after`. What
raises KeyError, TypeError, ValueError or ArithmeticError there is refused as
malformed; an AttributeError escapes as an internal error.
"""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
import kassiber  # noqa: F401  (the 32-digit decimal context)
from kassiber.core import review_workflow as review

DESTINATION = ROOT / "crates/transfers/tests/review_delta.json"


def effects(*years, **extra):
    return {"tax": {"fiat_currency": "EUR", "jurisdiction": None, "years": list(years),
                    "years_omitted": 0, **extra}}


def year(value, realized="0.00", income="0.00", **extra):
    return {"year": value, "realized_gain_loss": realized, "income": income, **extra}


def austrian(value, realized="0.00", income="0.00", kennzahlen=None, status="indicative",
             cents=0, withheld=()):
    return year(value, realized, income, kennzahlen=kennzahlen or {},
                indicative_tax={"status": status, "special_rate_base_eur_cents": 0,
                                "estimated_tax_eur_cents": cents, "withheld_by": list(withheld)})


def inputs():
    yield effects(), effects()
    yield effects(year(2024, "10.00")), effects(year(2024, "10.00"))
    yield effects(year(2024, "10.00")), effects(year(2024, "12.50", "1.00"))
    yield effects(year(2023, "1.00")), effects(year(2024, "2.00"))
    yield effects(year(2024, "0.00")), effects(year(2024, "0.004"))
    yield effects(year(2024, "0.00")), effects(year(2024, "0.005"))
    yield effects(year(2024, "0.005")), effects(year(2024, "0.00"))
    yield effects(year(2024, "-0.004")), effects(year(2024, "0.00"))
    yield effects(year(2024, "1" * 31 + ".99")), effects(year(2024, "0"))
    yield effects(year(2024, "9" * 33)), effects(year(2024, "0"))
    yield effects(year(2024, "1E+40")), effects(year(2024, "0"))
    yield (effects(austrian(2023, kennzahlen={"174": 1000}, cents=275)),
           effects(austrian(2023, kennzahlen={"174": 1500, "176": 200}, cents=358)))
    yield (effects(austrian(2023, kennzahlen={"174": 1000}, cents=275)),
           effects(austrian(2023, kennzahlen={"174": 1000}, cents=275)))
    yield (effects(austrian(2023, kennzahlen={"174": 1000}, cents=275)),
           effects(austrian(2023, kennzahlen={"174": 1000}, status="withheld", cents=None,
                            withheld=("quarantine",))))
    yield (effects(austrian(2021, kennzahlen={"801": 5}, status="not_applicable", cents=None)),
           effects(austrian(2021, kennzahlen={"801": 5}, status="not_applicable", cents=None)))
    yield (effects(austrian(2025, kennzahlen={"172": 1, "801": 2})),
           effects(year(2025, "1.00")))
    yield effects(year(2025)), effects(austrian(2025, kennzahlen={"10": 1, "9": 2, "801": 3}))
    yield (effects(austrian(2025, kennzahlen={"174": 7}, cents=2)),
           effects(austrian(2025, kennzahlen={"174": 7}, status="indicative", cents=2)))
    # Only the estimate moves (the same base, other report blockers lifted).
    yield (effects(austrian(2025, kennzahlen={"174": 7}, cents=2)),
           effects(austrian(2025, kennzahlen={"174": 7}, cents=3)))
    # Only the status moves.
    yield (effects(austrian(2025, kennzahlen={"174": 7}, status="withheld", cents=None)),
           effects(austrian(2025, kennzahlen={"174": 7}, status="not_applicable", cents=None)))
    # Untrusted shapes.
    for value in (None, [], "", 7, {}, {"tax": None}, {"tax": {}}, {"tax": {"years": None}},
                  {"tax": {"years": 7}}, {"tax": {"years": "ab"}}, {"tax": {"years": {"2024": 1}}},
                  {"tax": {"years": [{}]}}, {"tax": {"years": [[2024]]}}, {"tax": {"years": [None]}}):
        yield value, effects()
        yield effects(), value
    yield effects(year([2024])), effects()
    yield effects(year({"y": 1})), effects()
    yield effects(year(None)), effects()
    yield effects(year(None)), effects(year(2024))
    yield effects(year("2024"), year("2023", "1")), effects()
    yield effects(year("2024")), effects(year(2024))
    yield effects(year(2024.0, "1")), effects(year(2024, "2"))
    yield effects(year(2024, "2")), effects(year(2024.0, "1"))
    yield effects(year(True, "1")), effects(year(1, "2"))
    yield effects(year(2024.5, "1")), effects(year(2024, "2"))
    yield effects(year(2024, "1"), year(2024, "5")), effects(year(2024, "5"))
    for amount in (None, [], {}, [0, [1], -2], "abc", "", " 1.5 ", "1_0", "١٢", "NaN", "-NaN007",
                   "NaN" + "9" * 40, "sNaN", "Infinity", "-inf", 1.1, -0.0, 3, True, 10 ** 40, 1e300):
        yield effects(year(2024, amount)), effects(year(2024, "1.00"))
        yield effects(year(2024, "1.00")), effects(year(2024, amount))
    yield effects(year(2024, "NaN")), effects(year(2024, "Infinity"))
    yield effects(year(2024, "Infinity")), effects(year(2024, "-NaN5"))
    for kennzahlen in ([], [172], ["172"], "17", "", None, 7, {"x": 1}, {"172": "a"}, {"172": None},
                       {"172": 1.5}, {"172": True}, {"172": [1]}, {" 172 ": 1, "172": 2}, {"1_72": 3},
                       {"+172": 1, "172": 1}, {"9" * 400: 1}):
        yield effects(austrian(2024, kennzahlen={"174": 1})), effects(year(2024, kennzahlen=kennzahlen))
        yield effects(year(2024, kennzahlen=kennzahlen)), effects(austrian(2024, kennzahlen={"174": 1}))
        yield effects(year(2024, kennzahlen=kennzahlen)), effects(year(2024, kennzahlen=kennzahlen))
    for tax in ([], [1], "", "x", 0, 1, 2.5, True, False, None, {}, {"status": None},
                {"estimated_tax_eur_cents": "1"}, {"estimated_tax_eur_cents": 1.5},
                {"estimated_tax_eur_cents": True}, {"estimated_tax_eur_cents": [1]},
                {"estimated_tax_eur_cents": 10 ** 400}, {"status": 1}, {"status": True}):
        yield (effects(austrian(2024, cents=5)),
               effects(year(2024, kennzahlen={}, indicative_tax=tax)))
        yield (effects(year(2024, kennzahlen={}, indicative_tax=tax)),
               effects(austrian(2024, cents=5)))
    yield effects(year(2024, indicative_tax=[1])), effects(year(2024, "1"))


def outcome(before, after):
    try:
        return {"delta": review._tax_delta(before, after)}
    except (KeyError, TypeError, ValueError, ArithmeticError):
        return {"fault": "malformed"}
    except AttributeError as error:
        return {"fault": "internal", "message": str(error)}


def export():
    cases = [dict(before=before, after=after, **outcome(before, after)) for before, after in inputs()]
    return json.dumps(cases, ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = export()
    if args.check:
        if DESTINATION.read_text() != encoded:
            raise SystemExit("review delta fixtures differ from current Python")
    else:
        DESTINATION.write_text(encoded)
    print(f"{len(json.loads(encoded))} review delta cases")
