"""Raw-fact component parity, branch accounting, refusals, and permutations."""
from collections import Counter
import json
from tests.custody_engine_compare import python_refusal

from kassiber.core.custody_compute import compute_components_stage
from tests.custody_engine_compare import assert_same, permuted_snapshot, rust_results
from tests.custody_snapshot_generator import COMPONENT_SCENARIOS, component_snapshot


def stage(book):
    components, blockers = compute_components_stage(book)
    return {"components": components, "blockers": blockers}


def test_two_thousand_generated_component_snapshots_and_permutations():
    counts = Counter()
    for seed in range(2000):
        book = component_snapshot(seed)
        expected = stage(book)
        actual, _ = rust_results(book, finalized=False)
        assert_same(expected, actual["components"], f"component seed {seed}")
        shuffled = permuted_snapshot(book)
        # Component native-support reads sort their own population. The full
        # pipeline preserves this order for the evidence stage's row output.
        shuffled["transactions"].reverse()
        permuted_stage = stage(shuffled)
        assert_same(expected, permuted_stage, f"Python component permutation {seed}")
        actual, _ = rust_results(shuffled, finalized=False)
        assert_same(expected, actual["components"], f"Rust component permutation {seed}")
        counts["books"] += 1
        case = COMPONENT_SCENARIOS[seed % len(COMPONENT_SCENARIOS)]
        counts["component:" + case] += 1
        counts[f"scenario:{case}:active"] += sum(c["effective_state"] == "active" for c in expected["components"])
        counts[f"scenario:{case}:blocked"] += len(expected["blockers"])
        counts[f"scenario:{case}:inactive"] += len(book["components"]) - len(expected["components"])
        for component in expected["components"]:
            counts["effective:" + component["effective_state"]] += 1
            counts["evidence:" + component["evidence_status"]["status"]] += 1
            counts["native:" + component["native_support_status"]["status"]] += 1
            for issue in component["validation"]["issues"]:
                counts["issue:" + issue["code"]] += 1
    assert counts["effective:active"] > 0
    assert all(counts["component:" + case] >= 2000 // len(COMPONENT_SCENARIOS) for case in COMPONENT_SCENARIOS)
    assert all(counts["issue:" + code] for code in (
        "active_lineage_conflict", "active_transaction_membership_conflict",
        "component_leg_count_mismatch", "anchor_transaction_missing", "anchor_transaction_excluded",
        "unbalanced_quantity", "allocation_source_coverage_mismatch", "allocation_sink_coverage_mismatch",
        "custody_network_scope_laundering", "custody_location_continuity_mismatch",
        "revision_link_missing", "revision_link_invalid", "component_native_support_contradicted",
        "allocation_chronology_mismatch",
    )), counts
    print("component generator branches: " + json.dumps(dict(sorted(counts.items())), sort_keys=True))


def test_checked_in_component_vectors_and_refusal_codes():
    from tests.custody_component_vectors import PATH, semantics, vectors
    import kassiber_tax
    expected = json.loads(PATH.read_text())
    assert vectors() == expected
    assert json.loads(PATH.with_name("custody_component_semantics.json").read_text()) == semantics()
    for vector in expected:
        for book in (vector["input"], permuted_snapshot(vector["input"])):
            response = json.loads(kassiber_tax.custody(json.dumps(book), finalized=False))
            if "error_code" in vector:
                assert response["error"]["code"] == vector["error_code"], vector["case"]
                assert response["error"]["stage"] == python_refusal(book, finalized=False)[1]
            else:
                assert_same(vector["components"], response["stages"]["components"], vector["case"])
