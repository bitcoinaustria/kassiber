#!/usr/bin/env python3
"""Freeze CPython 3.11 timestamp and legacy-price formatting boundary vectors."""
import argparse
import json
from pathlib import Path
import random
import struct
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.errors import AppError
from kassiber.time_utils import _iso_z, _parse_iso_datetime


def content():
    assert sys.version_info[:2] == (3, 11), "use locked Python 3.11"
    component = json.loads((ROOT / "tests/fixtures/custody_component_semantics.json").read_text())
    inputs = [row["input"] for row in component["times"] if isinstance(row["input"], str)]
    inputs.extend(("2025-01-01Z", "2025-W01Z", "2025-01-01T01:02:03.1234567+01:02:03.456789", "2025-01-01T01:02:03+00:00:00.123456", "", " "))
    times = []
    for raw in dict.fromkeys(inputs):
        try:
            parsed = _parse_iso_datetime(raw, "start")
            times.append({"input": raw, "value": _iso_z(parsed) if parsed else "", "code": None})
        except (AppError, OverflowError) as exc:
            times.append({"input": raw, "value": str(exc), "code": exc.code if isinstance(exc, AppError) else "internal_error"})
    rng = random.Random(329)
    floats = [0.0, -0.0, 1.000000000001, 1e-4, 1e-5, 1e11, 1e12, 9.999999999995, 999999999999.5]
    floats.extend(struct.unpack("!d", rng.getrandbits(64).to_bytes(8, "big"))[0] for _ in range(2000))
    floats = [n for n in floats if abs(n) != float("inf") and n == n]
    return json.dumps({"timestamps": times, "floats": [{"value": n, "formatted": format(n, ".12g")} for n in floats]}, ensure_ascii=True, indent=2) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = ROOT / "crates/transactions/tests/data/metadata-vectors.json"
    generated = content()
    if args.check:
        if path.read_text() != generated:
            raise SystemExit("metadata vectors drifted")
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(generated)
