"""Independent interpreter slices; test-only and raw snapshot in both engines.

This does not declare interpretation implemented. Until composition is ported,
each slice runs in isolation before the other interpreters reserve or hold rows.
"""
from __future__ import annotations

from functools import lru_cache
import json
from pathlib import Path
import subprocess

from kassiber.core import custody_interpreters as reference
from kassiber.core.custody_compute import compute_components_stage, compute_evidence_stage
from tests.custody_snapshot import to_jsonable

ROOT = Path(__file__).resolve().parents[1]


def python_results(snapshot):
    components, _ = compute_components_stage(snapshot)
    canonical = compute_evidence_stage(snapshot)["canonical_input"]
    observations = reference._observations_by_transaction(canonical)
    excluded = sorted({
        str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
        for component in components for leg in component.get("legs", ())
        if leg.get("anchor_transaction_id") or leg.get("transaction_id")
    })
    return to_jsonable({"shared": {
        "observations_by_transaction": observations,
        "excluded_transaction_ids": excluded,
        "row_anchor_ids": [reference._anchor_id(row) for row in snapshot["transactions"]],
    }}, redact_config=False)


@lru_cache(maxsize=1)
def executable():
    subprocess.run(["cargo", "build", "--locked", "-q", "-p", "kassiber-custody", "--example", "interpretation_partial"], cwd=ROOT, check=True)
    return ROOT / "target/debug/examples/interpretation_partial"


def rust_results(snapshot):
    result = subprocess.run([str(executable())], input=json.dumps(snapshot) + "\n", capture_output=True, text=True, check=True)
    value = json.loads(result.stdout)
    if "partial_error" in value:
        raise AssertionError("Rust partial interpretation refused the reference snapshot")
    return value


def compare(snapshot):
    from tests.custody_engine_compare import assert_same
    expected, actual = python_results(snapshot), rust_results(snapshot)
    assert_same(expected, actual, "partial interpretation")
    return tuple(expected)
