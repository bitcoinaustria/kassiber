"""Composed holds/privacy controls with independent scenario counts."""
from collections import Counter
import json

from tests.custody_engine_compare import rust_results
from tests.custody_interpretation_stage import python_stage
from tests.custody_snapshot import to_jsonable
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


def test_safety_reasons_in_both_complete_stages():
    assert set(COUNTS) == set(SAFETY_SCENARIOS)
    for seed,case in enumerate(SAFETY_SCENARIOS):
        book=safety_snapshot(seed)
        expected=to_jsonable(python_stage(book),redact_config=False)
        actual=rust_results(book)[0]["interpretation"]
        for engine,result in (("Python",expected),("Rust",actual)):
            assert Counter(q["reason"] for q in result["quarantines"]) == expected_reasons(case),(case,engine)


def test_safety_frozen_helpers():
    from tests.custody_safety_vectors import PATH, helpers
    assert json.loads(PATH.read_text()) == helpers()
