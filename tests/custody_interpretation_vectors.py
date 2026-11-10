"""Frozen isolated interpreter outputs computed only by the Python reference."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from tests.custody_interpretation_partial import python_results
from tests.custody_snapshot_generator import component_snapshot, snapshot, SCENARIOS

PATH = Path(__file__).parent / "fixtures/custody_interpretation_cases.json"


def books():
    yield "shared_aliases", snapshot(SCENARIOS.index("duplicate_slot"))
    yield "shared_reservations", component_snapshot(0)
    book = deepcopy(snapshot(0))
    book["transactions"][0]["journal_transaction_id"] = "original-anchor"
    yield "shared_anchor_alias", book
    from tests.custody_interpretation_generator import SAMOURAI_SCENARIOS, samourai_snapshot
    for seed, case in enumerate(SAMOURAI_SCENARIOS):
        yield "samourai_" + case, samourai_snapshot(seed, case=case)
    yield "samourai_float_overflow", samourai_snapshot(0, case="float_overflow")
    from tests.custody_interpretation_generator import RECORDED_SCENARIOS, recorded_snapshot
    for seed, case in enumerate(RECORDED_SCENARIOS):
        yield "recorded_" + case, recorded_snapshot(seed, case=case)
    from tests.custody_fanout_generator import FANOUT_SCENARIOS, fanout_snapshot
    for seed, case in enumerate(FANOUT_SCENARIOS):
        yield "fanout_" + case, fanout_snapshot(seed, case=case)

    from tests.custody_ownership_vectors import books as ownership_books
    included = {"bitcoin_ownership", "liquid_ownership", "liquid_rawhex", "liquid_rawhex_short_locktime", "liquid_rawhex_flag", "liquid_rawhex_nonminimal", "source_missing", "source_missing_untrusted", "source_zero_delta", "reuse", "reuse_two_exact", "reuse_near", "reuse_different_tx", "reuse_no_ref", "synthetic_inbound", "foreign_input", "historical_fallback", "wrong_input_network", "ordinary", "change", "ambiguous_external", "unknown_owned_value", "unknown_input_value", "asset_unknown_input", "asset_other_external", "conflicts", "liquid_change", "inclusive_exact", "inclusive_unknown", "implicit", "residual", "two_destinations", "destination_order", "destination_aggregate", "destination_missing_ref", "unbounded_index", "pricing", "rollback", "consumed"}
    for case, book, handled in ownership_books():
        if case in included:
            assert not handled
            yield "ownership_" + case, book

    from tests.custody_consolidation_vectors import books as consolidation_books
    included = {"bitcoin", "liquid", "different_fees", "collaborative", "collab_stale", "collab_residual", "collab_missing_wallet", "foreign_input", "change", "shared_change", "external", "ambiguous_destination", "no_destination", "missing_wallet", "duplicate_receipt", "wrong_receipt", "zero_receipt", "synthetic_receipts", "two_destinations", "two_synthetic", "mixed_destinations", "sink_order", "sender_order", "split_fee", "exact_msat", "offgroup_near", "offgroup_different", "value_pricing", "exact_pricing", "duplicate_wallet_ref", "liquid_component_display"}
    for case, book, handled in consolidation_books():
        if case in included:
            assert not handled
            yield "consolidation_" + case, book

    from tests.custody_pipeline_generator import PIPELINE_SCENARIOS,pipeline_snapshot
    for seed,case in enumerate(PIPELINE_SCENARIOS):
        yield 'pipeline_'+case,pipeline_snapshot(seed,case=case)

    from tests.custody_pipeline_vectors import books as pipeline_books
    from kassiber.core.chain_observer.provenance import canonical_graph_hash,canonical_observed_quantity_hash
    for case,book,_handled in pipeline_books():
        if case in {'fanout_graphless_source','consolidation_pricing_error'}:
            for row in book['transactions']:
                row['observation_graph_hash']=canonical_graph_hash(row['raw_json'])
                row['observation_quantity_hash']=canonical_observed_quantity_hash(row)
            yield 'pipeline_'+case,book

    from tests.custody_safety_generator import SAFETY_SCENARIOS, safety_snapshot
    for seed, case in enumerate(SAFETY_SCENARIOS):
        yield "safety_" + case, safety_snapshot(seed, case=case)

    from tests.custody_channel_generator import CHANNEL_SCENARIOS, channel_snapshot
    for seed,case in enumerate(CHANNEL_SCENARIOS):
        yield "channel_"+case,channel_snapshot(seed)
    yield "channel_role_reservation",channel_snapshot(0,case="role_reservation")
    malformed=channel_snapshot(0)
    malformed["channel_records"][1]["raw_json"]=json.dumps(dict(_kassiber_provenance=dict(import_source=[])))
    yield "channel_unhashable_provenance",malformed

    from tests.custody_native_generator import NATIVE_SCENARIOS, native_snapshot
    for seed, case in enumerate(NATIVE_SCENARIOS):
        yield "native_"+case,native_snapshot(seed,case=case)


def vectors():
    return [{"case": case, "input": book, "partial": python_results(book)} for case, book in books()]


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True, sort_keys=True) + "\n"
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
