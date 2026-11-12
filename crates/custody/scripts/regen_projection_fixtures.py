#!/usr/bin/env python3
"""Regenerate the private finalized-projection fixtures from the Python reference.

The fixtures carry rounds 5 and 8 (valuation holds, the 1e19 price limit and
retained price causes) and main #708's `pricing_review_price`. Run from the
repository root:

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/custody/scripts/regen_projection_fixtures.py [--check]
"""
from collections import Counter
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2].parent
sys.path.insert(0, str(ROOT))

from tests import custody_projection_adapter as adapter  # noqa: E402
from tests import custody_projection_composition as composition  # noqa: E402
from tests import custody_projection_primitives as primitives  # noqa: E402
from tests import custody_projection_quarantines as quarantines  # noqa: E402
from tests.test_custody_projection_composition_port import activity as composition_activity  # noqa: E402
from tests.test_custody_projection_quarantines_port import activity as quarantine_activity  # noqa: E402
from tests.test_custody_projection_stage import expected as stage_expected  # noqa: E402

def frozen(module):
    return [dict(v, **module.evaluate(v)) for v in module.operands()]


def counts(module, activity):
    result = Counter()
    for v in module.operands():
        for i in range(20):
            result[v["case"]] += activity(module.evaluate(module.scenario(v, i)))
    return dict(result)


def primitive_counts():
    result = Counter()
    for i in range(len(list(primitives.operands()))):
        seen = set()
        for variant in range(20):
            case, values, results = primitives.scenario(i, variant)
            key = json.dumps(values, sort_keys=True)
            if key in seen:
                continue
            seen.add(key)
            result[case] += sum(map(primitives.activity, results))
    return dict(result)


def outputs():
    fixtures = ROOT / "tests/fixtures"
    yield fixtures / "custody_projection_composition.json", json.dumps(
        frozen(composition), ensure_ascii=True, sort_keys=True)
    yield fixtures / "custody_projection_quarantines.json", json.dumps(
        frozen(quarantines), ensure_ascii=True, indent=2)
    yield fixtures / "custody_projection_primitives.json", json.dumps(
        primitives.vectors(), ensure_ascii=True, sort_keys=True)
    yield fixtures / "custody_projection_adapter.json", json.dumps(
        adapter.vectors(), ensure_ascii=True, sort_keys=True)
    for name in ("custody_projection_stage.json", "custody_projection_admission.json"):
        path = fixtures / name
        cases = [dict(case, expected=stage_expected(case["input"]))
                 for case in json.loads(path.read_text())]
        yield path, json.dumps(cases, ensure_ascii=True, separators=(",", ":"))
    for name, value in [
        ("custody_projection_composition_counts.json", counts(composition, composition_activity)),
        ("custody_projection_quarantines_counts.json", counts(quarantines, quarantine_activity)),
        ("custody_projection_counts.json", primitive_counts()),
    ]:
        path = fixtures / name
        previous = path.read_text()
        indent = 2 if previous.startswith("{\n") else None
        sort = list(json.loads(previous)) == sorted(json.loads(previous))
        yield path, json.dumps(value, ensure_ascii=True, indent=indent, sort_keys=sort)


def main():
    check = "--check" in sys.argv
    drifted = []
    for path, text in outputs():
        text += "\n"
        if path.read_text() != text:
            drifted.append(path.name)
            if not check:
                path.write_text(text)
    print(("drifted: " if check else "rewrote: ") + (", ".join(drifted) or "none"))
    return 1 if check and drifted else 0


if __name__ == "__main__":
    raise SystemExit(main())
