"""Test-only projection of the Python native producer's consumed candidate fields."""
from kassiber.core.custody_native_transitions import compile_native_transitions, has_native_transition_evidence
from kassiber.core.transfer_matching import suggest_swap_candidates

FIELDS = ('out_id','in_id','confidence','method','default_kind','swap_fee_msat','out_occurred_at','conflict_size','conflict_set_id')
def candidate_projection(candidate):
    return {key:getattr(candidate,key) for key in FIELDS}

def native_results(snapshot, observations):
    try:
        present=has_native_transition_evidence(snapshot['transactions'])
        pairs,proofs,quarantines=compile_native_transitions(snapshot['transactions'],observations,profile=snapshot['profile'],occupied_ids=set(),dismissals=snapshot['pair_dismissals'],as_of=snapshot['as_of']) if present else ([],{},())
        return dict(has_evidence=present,pairs=pairs,proofs=[dict(key=key,proof=dict(candidate=candidate_projection(p.candidate),source_quantity_hash=p.source_quantity_hash,target_quantity_hash=p.target_quantity_hash,policy=p.policy)) for key,p in proofs.items()],quarantines=quarantines,candidates=[candidate_projection(c) for c in suggest_swap_candidates(snapshot['transactions'],include_heuristics=False,now_iso=snapshot['as_of'])] if present else [])
    except Exception as exc:
        return dict(error=type(exc).__name__)
