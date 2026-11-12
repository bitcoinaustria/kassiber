"""CPython oracle for the private canonical custody edge primitive."""
from dataclasses import replace
from types import SimpleNamespace
import json
from pathlib import Path

from kassiber.core import custody_quantity as q
from kassiber.core.custody_quantity_runtime import canonical_internal_transfer_rows
from tests.custody_snapshot import to_jsonable
from tests.test_custody_quantity import _observation

PATH = Path(__file__).parent / "fixtures/custody_quantity_transfers.json"
SCENARIOS = ("verified", "reviewed", "blocked", "retained", "external", "labels",
             "empty_ids", "anchor_ids", "liquid", "wide", "slice", "sort")


def scenario(seed):
    name = SCENARIOS[seed % len(SCENARIOS)]
    amount = 23 if name in {"sort", "slice"} else 10**100 + seed if name == "wide" else 100 + seed
    source = _observation("out", "old", "outbound", amount)
    target = _observation("in", "new", "inbound", amount)
    if name == "anchor_ids":
        source = replace(source, anchor_transaction_id="source-anchor")
        target = replace(target, anchor_transaction_id="target-anchor", occurred_at="2027-01-01T00:00:00Z")
    if name == "liquid":
        source = replace(source, event_key=replace(source.event_key, network="liquidtestnet", chain="liquid"), asset="LBTC")
        target = replace(target, event_key=replace(source.event_key, chain="bitcoin", network="test"), asset="BTC")
    reviewed = name in {"reviewed", "retained"}
    claim = q.QuantityClaim("edge", q.QuantitySlice(source.quantity_hash, 0, amount),
        q.INTERNAL_REVIEWED if reviewed else q.INTERNAL_VERIFIED,
        q.ClaimPriority.REVIEWED_COMPONENT if reviewed else q.ClaimPriority.EXACT_NATIVE_EVENT,
        "known-edge", allow_cross_rail=name == "liquid", target=None if name == "retained" else q.QuantitySlice(target.quantity_hash, 0, amount),
        destination_kind="retained_custody" if name == "retained" else None,
        atomic_bundle_id="bundle" if reviewed else None,
        component_id="component" if reviewed else None)
    claims = [] if name == "external" else [claim]
    if name in {"sort", "slice"}:
        # A lower-priority overlapping hold creates three valid selected native
        # slices (10, 2, 11 msat) sharing one claim id. No synthetic decisions.
        claims.append(q.QuantityClaim("boundary", q.QuantitySlice(source.quantity_hash, 10, 12),
            q.CUSTODY_SUSPENSE, q.ClaimPriority.ACCOUNTING_CONVENTION, "boundary"))
    projection = q.project_quantities([source, target], claims)
    if name == "empty_ids":
        projection = replace(projection, decisions=tuple(replace(d, component_id="") for d in projection.decisions))
    # A separate exact edge keeps exclusion scenarios active as well.
    extra_source = _observation("extra-out", "extra-old", "outbound", 1)
    extra_target = _observation("extra-in", "extra-new", "inbound", 1)
    extra = q.project_quantities([extra_source, extra_target], [q.QuantityClaim(
        "extra", q.QuantitySlice(extra_source.quantity_hash, 0, 1), q.INTERNAL_VERIFIED,
        q.ClaimPriority.EXACT_NATIVE_EVENT, "companion",
        target=q.QuantitySlice(extra_target.quantity_hash, 0, 1))])
    projection = replace(projection,
        observations=projection.observations + extra.observations,
        decisions=projection.decisions + extra.decisions)
    eligible = () if name == "blocked" else projection.decisions[:1] if name == "slice" else projection.decisions
    refs = {"old": {"label": None}, "new": {"label": ""}} if name == "labels" else {}
    state = SimpleNamespace(projection=projection, tax_eligibility=SimpleNamespace(eligible_decisions=eligible))
    expected = to_jsonable(canonical_internal_transfer_rows(state, refs), redact_config=False)
    active = 0 if name in {"retained", "external"} else 3 if name in {"slice", "sort"} else 1
    active += 1
    assert len(expected) == active, name
    return name, dict(op="transfers", projection=to_jsonable(projection, redact_config=False),
        eligible=to_jsonable(eligible, redact_config=False), refs=refs), expected, active


def vectors():
    return [dict(case=name, **value, expected=expected, active=active)
            for name, value, expected, active in (scenario(i) for i in range(len(SCENARIOS)))]


if __name__ == "__main__":
    import sys
    data = vectors()
    if "--write" in sys.argv:
        PATH.write_text(json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n")
    else:
        assert json.loads(PATH.read_text()) == data
