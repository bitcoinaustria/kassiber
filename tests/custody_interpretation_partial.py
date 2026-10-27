"""Independent interpreter slices; test-only and raw snapshot in both engines.

This does not declare interpretation implemented. Until composition is ported,
each slice runs in isolation before the other interpreters reserve or hold rows.
"""
from __future__ import annotations

from functools import lru_cache
import json
import os
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
    present = any(row.get("config_text_mentions_samourai", "samourai" in str(row.get("config_json") or "").lower()) for row in snapshot["transactions"])
    pairs, fees, candidates, touched = reference._samourai_privacy_pairs(snapshot["transactions"], observations) if present else ([], [], set(), set())
    claims, blocked, quarantines = reference._pair_claims(pairs, observations,
        excluded_transaction_ids=set(excluded), wallet_refs_by_id={}, physical_scopes_by_anchor={})
    assert not blocked and not quarantines  # Samourai pairs use only same-event proof.
    unverified = sorted(candidates - touched - set(excluded))
    rows_by_id = {str(row.get("id") or ""): row for row in snapshot["transactions"]}
    quarantines = [{
        "transaction_id": transaction_id,
        "workspace_id": rows_by_id.get(transaction_id, {}).get("workspace_id"),
        "profile_id": rows_by_id.get(transaction_id, {}).get("profile_id"),
        "reason": "samourai_native_event_unverified",
        "detail_json": json.dumps({"required_for": "authoritative_same_event_observation", "resolution": "sync every imported Samourai source with a chain observer"}, sort_keys=True),
    } for transaction_id in unverified]
    return to_jsonable({"samourai": {
        "pairs": pairs, "fee_claims": fees, "candidate_ids": sorted(candidates), "touched_ids": sorted(touched),
        "claims": (*claims, *fees), "blocked_transaction_ids": unverified, "quarantines": quarantines,
    }, "shared": {
        "observations_by_transaction": observations,
        "excluded_transaction_ids": excluded,
        "row_anchor_ids": [reference._anchor_id(row) for row in snapshot["transactions"]],
    }}, redact_config=False)


@lru_cache(maxsize=1)
def executable():
    subprocess.run(["cargo", "build", "--locked", "-q", "-p", "kassiber-custody", "--example", "interpretation_partial"], cwd=ROOT, check=True, timeout=300)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    return target / "debug/examples" / ("interpretation_partial.exe" if os.name == "nt" else "interpretation_partial")


def rust_results(snapshot):
    result = subprocess.run([str(executable())], input=json.dumps(snapshot) + "\n", capture_output=True, text=True, check=True, timeout=60)
    value = json.loads(result.stdout)
    if "partial_error" in value:
        raise AssertionError("Rust partial interpretation refused the reference snapshot")
    return value


def compare(snapshot):
    from tests.custody_engine_compare import assert_same
    expected, actual = python_results(snapshot), rust_results(snapshot)
    assert_same(expected, actual, "partial interpretation")
    return tuple(expected)
