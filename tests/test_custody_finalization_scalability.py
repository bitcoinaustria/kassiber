"""Bound target lookup work at the production custody-to-tax boundary."""
from dataclasses import replace
from unittest.mock import patch

from kassiber.core.custody_tax_projection import compile_finalized_tax_projection
from tests.custody_tax_helpers import finalized_tax_inputs
from tests.test_custody_native_transitions import PROFILE, native_transition_rows


def native_moves(count):
    base, refs = native_transition_rows()
    rows = [{**base[0], "amount": count * 100_000_000}]
    refs["receiver"] = {**refs["lightning-wallet"], "id": "receiver", "label": "Receiver"}
    for index in range(count):
        source = {**base[1], "id": f"out-{index}", "external_id": f"pay-{index}", "payment_hash": f"{index+1:064x}", "fee": 0}
        target = {**source, "id": f"in-{index}", "external_id": f"invoice-{index}", "direction": "inbound", "wallet_id": "receiver", "kind": "lnd_invoice"}
        rows.extend((source, target))
    captured = []
    def capture(*args, **kwargs):
        captured.append((args, kwargs))
        return compile_finalized_tax_projection(*args, **kwargs)
    with patch("tests.custody_tax_helpers.compile_finalized_tax_projection", side_effect=capture):
        finalized_tax_inputs(PROFILE, rows=rows, wallet_refs_by_id=refs)
    return captured[0]


class CountTargetReads:
    """Instrument decisions without replacing projection or quantity behavior."""
    def __init__(self, decision, counter):
        self.decision, self.counter = decision, counter

    def __getattr__(self, name):
        if name == "target":
            self.counter[0] += 1
        return getattr(self.decision, name)


def test_finalization_target_lookup_is_linear_and_preserves_native_moves():
    count = 64
    args, kwargs = native_moves(count)
    profile, rows, state = args
    expected = compile_finalized_tax_projection(*args, **kwargs)
    reads = [0]
    counted = tuple(CountTargetReads(decision, reads) for decision in state.projection.decisions)
    state = replace(state, projection=replace(state.projection, decisions=counted))
    actual = compile_finalized_tax_projection(profile, rows, state, **kwargs)
    assert actual == expected
    assert len(actual.intra_pairs) == count
    assert len(actual.rows) == 2 * count + 1
    assert not actual.quarantines
    assert reads[0] <= 30 * count, f"{reads[0]} target reads for {count} independent moves"
