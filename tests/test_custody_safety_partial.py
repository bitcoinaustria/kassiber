"""Composed holds/privacy controls with independent scenario counts."""
from collections import Counter
import json

from tests.custody_engine_compare import assert_same, permuted_snapshot
from tests.custody_interpretation_partial import compare, python_results, rust_results
from tests.custody_safety_generator import SAFETY_SCENARIOS, safety_snapshot

# (raw held rows, surviving pairs, claims, rowless audits, transient rows).
COUNTS = {
    "pending_pair": (2,0,0,0,2), "pending_fanout": (3,0,0,0,3),
    "pending_consolidation": (3,0,0,0,3), "pending_ownership": (1,0,0,0,1),
    "replaced_spend": (2,1,1,0,2), "conflict_none_confirmed": (4,0,0,0,4),
    "conflict_both_confirmed": (4,0,0,0,4), "unscoped_provider": (2,0,0,0,0),
    "blank_identity": (0,0,0,0,0), "unresolved_fanout": (3,0,0,0,0),
    "unresolved_consolidation": (3,0,0,0,0), "unresolved_many_many": (4,0,0,0,0),
    **{name: (0,0,0,0,0) for name in ("privacy_coinjoin","privacy_collaborative","privacy_payjoin","privacy_payment_in_coinjoin","privacy_sweep")},
    "privacy_exact_pair": (0,1,1,0,0), "privacy_fanout": (0,2,2,0,0),
    "privacy_consolidation": (0,2,2,0,0), "privacy_untrusted_pair": (0,1,0,0,0),
    "privacy_rowless": (0,1,0,1,0), "confirmed_pair": (0,1,1,0,0),
    "status_string": (0,1,1,0,0), "legacy_missing_status": (0,1,1,0,0),
    "privacy_unknown": (0,0,0,0,0), "closed_newer_pending": (2,0,0,0,2),
    "closed_newer_confirmed": (0,1,1,0,0), "closed_equal_disagreement": (2,0,0,0,2),
    "closed_invalid_timestamp": (0,1,1,0,0), "liquid_pending": (2,0,0,0,2),
    "liquid_privacy_pair": (0,1,1,0,0), "samourai_unverified": (0,2,0,0,0),
    "samourai_verified": (0,2,1,0,0), "normalized_fee_only": (0,2,2,0,0),
    "privacy_reviewed": (0,0,0,0,0), "dependency_pending": (2,0,0,0,2),
    "dependency_privacy": (1,0,0,0,1), "lightning_chain_no_hash": (0,0,0,0,0),
}


def expected_reasons(case):
    if case == "dependency_pending": return Counter(pending_onchain_confirmation=1, transfer_pair_dependency_blocked=1)
    if case == "dependency_privacy": return Counter(pending_onchain_confirmation=1, privacy_hop_unresolved=1)
    if case.startswith("pending_") or case in {"closed_newer_pending", "closed_equal_disagreement", "liquid_pending"}:
        return Counter(pending_onchain_confirmation=COUNTS[case][0])
    if case in {"replaced_spend", "conflict_none_confirmed", "conflict_both_confirmed"}:
        return Counter(conflicting_spend=COUNTS[case][0])
    if case == "unscoped_provider": return Counter(unscoped_transfer_review=2)
    if case.startswith("unresolved_"): return Counter(owned_fanout_unresolved=COUNTS[case][0])
    if case in {"privacy_coinjoin","privacy_collaborative","privacy_payjoin","privacy_payment_in_coinjoin","privacy_sweep","privacy_rowless","privacy_reviewed"}:
        return Counter(privacy_hop_unresolved=1)
    if case == "privacy_untrusted_pair": return Counter(privacy_hop_unresolved=2, recorded_transfer_authority_conflict=1)
    if case == "samourai_unverified": return Counter(samourai_native_event_unverified=2, recorded_transfer_authority_conflict=1)
    return Counter()


def test_safety_generated_counts_and_permutations():
    assert set(COUNTS) == set(SAFETY_SCENARIOS)
    counts = {engine: {case: Counter() for case in SAFETY_SCENARIOS} for engine in ("Python", "Rust")}
    for seed in range(2000):
        case = SAFETY_SCENARIOS[seed % len(SAFETY_SCENARIOS)]
        book = safety_snapshot(seed)
        expected, actual = python_results(book), rust_results(book)
        assert_same(expected, actual, "composed safety")
        for engine, result in (("Python", expected["safety"]), ("Rust", actual["safety"])):
            assert "error" not in result, (seed, case, engine, result)
            observed = tuple(len(result[key]) for key in ("pair_inputs", "claims", "native_audits", "transient_blocked_transaction_ids"))
            assert (len(result["holds"]["blocked_ids"]), *observed) == COUNTS[case], (seed, case, engine, result)
            reasons = Counter(item["reason"] for item in result["quarantines"])
            assert reasons == expected_reasons(case), (seed, case, engine, reasons)
            counts[engine][case]["books"] += 1
            counts[engine][case]["held_books"] += bool(result["holds"]["blocked_ids"])
            counts[engine][case]["privacy_books"] += bool(result["privacy"]["blocked_transaction_ids"])
            counts[engine][case]["claim_books"] += bool(result["claims"])
            counts[engine][case]["audit_books"] += bool(result["native_audits"])
        if seed < len(SAFETY_SCENARIOS) * 2:
            compare(permuted_snapshot(book))
            from kassiber.core.custody_compute import compute_custody_decisions
            from tests.custody_snapshot import to_jsonable
            stages = {}
            compute_custody_decisions(book, stage_observer=lambda name, value: stages.setdefault(name, value))
            full = to_jsonable(stages["interpretation"], redact_config=False)
            for key in ("claims", "native_audits", "blocked_transaction_ids", "quarantines", "transient_blocked_transaction_ids"):
                assert_same(full[key], expected["safety"][key], "full Python safety " + key)
    for engine in counts:
        for index, case in enumerate(SAFETY_SCENARIOS):
            number = 2000 // len(SAFETY_SCENARIOS) + (index < 2000 % len(SAFETY_SCENARIOS))
            assert counts[engine][case]["books"] == number
            assert counts[engine][case]["held_books"] == (number if COUNTS[case][0] else 0)
            assert counts[engine][case]["privacy_books"] == (number if expected_reasons(case)["privacy_hop_unresolved"] else 0)
            assert counts[engine][case]["claim_books"] == (number if COUNTS[case][2] else 0)
            assert counts[engine][case]["audit_books"] == (number if COUNTS[case][3] else 0)
    assert counts["Python"] == counts["Rust"]
    print("Safety producer counts: " + json.dumps(counts, sort_keys=True))


def test_safety_frozen_helpers():
    from tests.custody_safety_vectors import PATH, helpers
    assert json.loads(PATH.read_text()) == helpers()
