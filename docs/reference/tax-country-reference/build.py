"""Write docs/reference/tax-country-<cc>-vectors.json from the reference calculators.

usage (from the repository root):
    python3 -I docs/reference/tax-country-reference/build.py . [de gb ch us]
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

META = {
    "de": {"currency": "EUR", "timezone": "Europe/Berlin", "tax_year": "calendar year, Europe/Berlin"},
    "gb": {"currency": "GBP", "timezone": "Europe/London", "tax_year": "6 April to 5 April, Europe/London"},
    "ch": {"currency": "CHF", "timezone": "Europe/Zurich", "tax_year": "calendar year (Steuerperiode), Europe/Zurich"},
    "us": {"currency": "USD", "timezone": "UTC", "tax_year": "calendar year, UTC"},
}


def main():
    root = Path(sys.argv[1])
    for cc in sys.argv[2:] or ["de", "gb", "ch", "us"]:
        calc = __import__(cc)
        vectors = __import__(f"{cc}_vectors").VECTORS
        ids = [v["id"] for v in vectors]
        assert len(ids) == len(set(ids)), "duplicate vector id"
        out = []
        for v in vectors:
            expected = calc.evaluate(v)
            hand = v.get("hand_check")
            if hand:
                calc.check_hand(v["id"], expected, hand)
            out.append({k: v[k] for k in v if k != "hand_check"} | {"expected": expected})
        doc = {
            "schema": "kassiber.tax-country-vectors/1",
            "country": cc,
            "registry_value": cc,
            "spec": f"docs/reference/tax-country-{cc}.md",
            "status": "preview: not reviewed by a tax professional for this country",
            "retrieved": "2026-11-07",
            **META[cc],
            "conventions": {
                "amounts": "decimal strings; fiat in the country currency, rounded half-even to cents after exact computation; quantities exact",
                "times": "ISO 8601 with offset; every date rule uses the local date in `timezone`",
                "expected": "computed by the spec's reference model; `refusal` means the implementation must fail closed with that code",
            },
            "vectors": out,
        }
        path = root / "docs" / "reference" / f"tax-country-{cc}-vectors.json"
        path.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n")
        print(cc, len(out), "vectors ->", path)


if __name__ == "__main__":
    main()
