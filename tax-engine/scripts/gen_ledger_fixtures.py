#!/usr/bin/env python3
"""Generate the projection-level tax-ledger goldens (plan 20, phase 4).

Each case is one ``build_ledger_state`` call: a profile and the finalized
custody projection it runs on (``inputs``), the frozen oracle's outcome on RP2
(``oracle``, ``tests/tax_oracle/``), and the outcome production must give
(``expected``). The cases are ``tests.tax_ledger_generator.corpus()``: named
histories for every shape the phase-4 engine must reproduce, plus seeded
books from the random generator. ``tests/tax_ledger_codec.py`` stores the
values, so a ``Decimal`` keeps its exponent and a tuple stays a tuple.

An outcome is the ledger with its generated fields masked
(``tests/tax_engine_compare.py``, ``comparable_ledger``) or the failure's
signature. ``oracle`` keeps the failure as RP2 raised it (its class names and
its configuration file prefix, with the random path replaced);
``expected`` applies the listed differences. ``listed_difference`` names the
one that applies: ``null`` when ``expected`` equals ``oracle``, ``LD-4`` when
only RP2 class names change, and ``LD-3`` when RP2's configuration prefix is
dropped too (every RP2 configuration failure is an RP2 class).

Before writing, the generator certifies every case: the inputs survive the
codec unchanged and give the same oracle outcome as the original inputs; they
violate none of the shapes a finalized projection cannot have
(``tests/tax_oracle/projection_range.py``); the oracle's outcome reaches the
shape the case is named for; and ``expected`` follows from ``oracle`` by the
listed differences alone. It refuses to change or drop a stored case's
``inputs``, ``oracle``, ``expected``, or ``listed_difference`` unless that
case is named with ``--accept``; adding cases never needs it. Production is
not run here;
``tests/test_tax_ledger_goldens.py`` checks it against ``expected`` and the
live oracle against ``oracle``.

    uv run --locked python tax-engine/scripts/gen_ledger_fixtures.py [--accept CASE ...]

It writes tax-engine/core/tests/data/ledger_fixtures.json, one case per line.
The output depends only on the corpus, the oracle, and the product modules
the oracle shares; ``--output`` writes elsewhere.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tests import tax_engine_compare as compare  # noqa: E402
from tests import tax_ledger_codec as codec  # noqa: E402
from tests.tax_ledger_generator import GOLDEN_SEEDS, SEED, GoldenCase, corpus  # noqa: E402
from tests.tax_oracle import projection_range  # noqa: E402

OUTPUT = REPO / "tax-engine" / "core" / "tests" / "data" / "ledger_fixtures.json"
SCHEMA = "kassiber-tax-engine/ledger-fixtures/v1"
# The fields a stored case keeps unless the case is accepted by name. The
# inputs are what the outcomes are of, so they cannot change silently either.
PROTECTED = ("inputs", "listed_difference", "oracle", "expected")
MAX_BYTES = 6_000_000


class CertificationError(Exception):
    pass


def _canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def listed_difference(oracle: dict[str, Any]) -> Optional[str]:
    """The listed difference that turns the stored oracle outcome into production's."""
    signature = oracle.get("error")
    if signature is None or compare.listed_signature(signature) == tuple(signature):
        return None
    messages_change = False
    while signature is not None:
        name, message, *fields = signature
        if compare.listed_signature((name, message))[1] != message:
            messages_change = True
        signature = fields[-1] if fields else None
    return "LD-3" if messages_change else "LD-4"


def certify(case: GoldenCase) -> dict[str, Any]:
    """The stored form of one case, after checking everything the goldens claim."""
    inputs = codec.encode_inputs(case.profile, case.inputs)
    profile, decoded = codec.decode_inputs(json.loads(_canonical(inputs)))
    if _canonical(codec.encode_inputs(profile, decoded)) != _canonical(inputs):
        raise CertificationError(f"{case.name}: the inputs do not survive the codec")
    violations = projection_range.violations(profile, decoded)
    if violations:
        raise CertificationError(f"{case.name}: the inputs have shapes a finalized projection cannot have: {violations}")
    outcome = compare.run_oracle(profile, decoded)
    complaints = case.check(*outcome)
    if complaints:
        raise CertificationError(f"{case.name}: the oracle does not reach the case's shape: {'; '.join(complaints)}")
    original = compare.run_oracle(case.profile, case.inputs)
    stored_differences = compare.ledger_differences(original, outcome)
    if stored_differences:
        raise CertificationError(f"{case.name}: the stored inputs change the oracle's outcome: {stored_differences[:5]}")
    oracle = compare.outcome_document(outcome, compare.oracle_error_signature)
    expected = compare.outcome_document(outcome, compare.error_signature)
    difference = listed_difference(oracle)
    # The §5.2 rule: production's outcome is the oracle's, after the listed
    # differences when (and only when) one is named.
    if difference is None:
        rule = compare.document_differences(oracle, expected)
    else:
        rule = compare.document_differences({"error": compare.listed_signature(oracle["error"])}, expected)
        if not compare.document_differences(oracle, expected):
            rule = [f"{difference} is named but changes nothing"]
    if rule:
        raise CertificationError(f"{case.name}: expected does not follow from oracle: {rule[:5]}")
    return {
        "name": case.name,
        "tags": list(case.tags),
        "listed_difference": difference,
        "inputs": inputs,
        "oracle": codec.encode(oracle),
        "expected": codec.encode(expected),
    }


def stored_cases(path: Path) -> dict[str, dict[str, Any]]:
    if not path.exists():
        return {}
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA:
        raise SystemExit(f"{path} is not a {SCHEMA} file")
    return {case["name"]: case for case in document["cases"]}


def refusals(stored: dict[str, dict[str, Any]], cases: list[dict[str, Any]], accept: set[str]) -> list[str]:
    """Stored outcomes this run would change or drop without ``--accept``."""
    found = []
    new = {case["name"]: case for case in cases}
    for name, old in stored.items():
        if name in accept:
            continue
        case = new.get(name)
        if case is None:
            found.append(f"{name}: the case is no longer generated")
            continue
        changed = [field for field in PROTECTED if _canonical(old.get(field)) != _canonical(case[field])]
        if changed:
            found.append(f"{name}: {', '.join(changed)} would change")
    return found


def render(cases: list[dict[str, Any]]) -> str:
    stats: dict[str, int] = {"cases": len(cases)}
    for case in cases:
        outcome = "errors" if "error" in case["oracle"] else "results"
        stats[outcome] = stats.get(outcome, 0) + 1
        if case["listed_difference"]:
            key = f"listed_{case['listed_difference']}"
            stats[key] = stats.get(key, 0) + 1
    header = {
        "schema": SCHEMA,
        "generator": "tax-engine/scripts/gen_ledger_fixtures.py",
        "seed": SEED,
        "golden_seeds": list(GOLDEN_SEEDS),
        "notes": [
            "Each case is one build_ledger_state call: inputs, the frozen oracle's outcome on RP2, and production's required outcome.",
            "Values use the tagged codec of tests/tax_ledger_codec.py; journal ids and the calculation engine are masked.",
            "oracle keeps RP2's failure classes and configuration prefix (its random path replaced); expected applies the listed differences.",
        ],
        "stats": dict(sorted(stats.items())),
    }
    head = _canonical(header)[:-1]
    lines = ",\n".join(_canonical(case) for case in cases)
    return f'{head},"cases":[\n{lines}\n]}}\n'


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--accept", action="append", default=[], metavar="CASE", help="let this case's stored outcome change or go")
    parser.add_argument("--max-bytes", type=int, default=MAX_BYTES, help="size budget (0 disables it)")
    args = parser.parse_args()

    stored = stored_cases(args.output)
    try:
        cases = [certify(case) for case in corpus()]
    except CertificationError as exc:
        raise SystemExit(f"certification failed: {exc}") from None
    accept = set(args.accept)
    unknown = sorted(accept - set(stored) - {case["name"] for case in cases})
    if unknown:
        raise SystemExit(f"--accept names unknown cases: {', '.join(unknown)}")
    refused = refusals(stored, cases, accept)
    if refused:
        raise SystemExit(
            "refusing to change stored outcomes (rerun with --accept CASE after reviewing each):\n  " + "\n  ".join(refused)
        )
    text = render(cases)
    size = len(text.encode())
    if args.max_bytes and size > args.max_bytes:
        raise SystemExit(f"ledger fixtures are {size} bytes, over the {args.max_bytes} byte budget")
    args.output.write_text(text, encoding="utf-8")
    added = sorted({case["name"] for case in cases} - set(stored))
    print(f"wrote {len(cases)} cases ({size} bytes, {len(added)} new) to {args.output}")


if __name__ == "__main__":
    main()
