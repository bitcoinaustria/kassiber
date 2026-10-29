"""Seeded primitive arbitration inputs, independent of the Rust implementation."""
from dataclasses import replace
import random
from kassiber.core import custody_quantity as q
from tests.test_custody_quantity import _observation
from tests.custody_snapshot import to_jsonable

SCENARIOS=("unmatched","fee_only","fee_inclusive","native","partial_native","source_conflict",
           "equal_priority","destination_conflict","atomic","atomic_partial","retained",
           "reviewed_external","external_fee","negative_balance_hold","domain_reject","cross_rail","one_msat")


def scenario(seed):
    case=SCENARIOS[seed%len(SCENARIOS)]
    rng=random.Random(seed)
    n=rng.choice((999,1000,1001,2**127+1,10**100+1))+rng.randrange(1000)
    if case=="one_msat":n=1
    out=_observation("out","old","outbound",n,fee_msat=0 if n==1 else 1)
    target=_observation("in","new","inbound",n)
    observations=[out,target]
    claims=[]
    def edge(identifier="edge", amount=n, priority=q.ClaimPriority.EXACT_NATIVE_EVENT, **fields):
        return q.QuantityClaim(identifier,q.QuantitySlice(out.quantity_hash,0,amount),
            q.INTERNAL_REVIEWED if priority==q.ClaimPriority.REVIEWED_COMPONENT else q.INTERNAL_VERIFIED,
            priority,"native",target=q.QuantitySlice(target.quantity_hash,0,amount),**fields)
    if case in {"native","one_msat"}:claims=[edge()]
    elif case=="partial_native":claims=[edge(amount=n//2)]
    elif case=="fee_only":observations=[replace(out,amount_msat=0,fee_msat=n)]
    elif case=="fee_inclusive":observations=[replace(out,amount_includes_fee=True),target]
    elif case=="source_conflict":
        claims=[edge(),replace(edge("reviewed",priority=q.ClaimPriority.REVIEWED_COMPONENT),component_id="review")]
    elif case=="equal_priority":claims=[edge(),edge("other")]
    elif case=="destination_conflict":
        other=_observation("other","another","outbound",n)
        observations.append(other)
        claims=[edge(),replace(edge("other"),source=q.QuantitySlice(other.quantity_hash,0,n))]
    elif case in {"atomic","atomic_partial"}:
        claims=[edge(amount=n//2,priority=q.ClaimPriority.REVIEWED_COMPONENT,atomic_bundle_id="bundle"),
                q.QuantityClaim("residual",q.QuantitySlice(out.quantity_hash,n//2,n),q.CUSTODY_SUSPENSE,
                    q.ClaimPriority.REVIEWED_COMPONENT,"reviewed residual",atomic_bundle_id="bundle")]
        if case=="atomic_partial":claims.append(edge("competing",amount=n//2))
    elif case in {"retained","reviewed_external","external_fee","negative_balance_hold"}:
        state=q.INTERNAL_REVIEWED if case=="retained" else q.CUSTODY_SUSPENSE if case=="negative_balance_hold" else q.EXTERNAL_CONFIRMED
        destination={"retained":"retained_custody","reviewed_external":"external","external_fee":"fee","negative_balance_hold":None}[case]
        claims=[q.QuantityClaim("review",q.QuantitySlice(out.quantity_hash,0,n),state,
            q.ClaimPriority.REVIEWED_COMPONENT,"negative_balance_hold" if case=="negative_balance_hold" else "review",
            destination_kind=destination,external_economic_subtype="gift" if case=="reviewed_external" else None)]
        if case=="negative_balance_hold":observations=[out]
    elif case in {"domain_reject","cross_rail"}:
        target=replace(target,asset="LBTC",event_key=replace(target.event_key,chain="liquid",network="liquidv1"))
        observations=[out,target];claims=[edge(allow_cross_rail=case=="cross_rail")]
    expected=q.project_quantities(observations,claims)
    return case,dict(observations=to_jsonable(observations,redact_config=False),claims=to_jsonable(claims,redact_config=False)),to_jsonable(expected,redact_config=False)


def signature(projection):
    from collections import Counter
    return ",".join(f"{key}:{value}" for key,value in sorted(Counter(d["state"] for d in projection["decisions"]).items())) or "empty"


if __name__=="__main__":
    import json
    from collections import Counter
    from pathlib import Path
    counts={case:Counter() for case in SCENARIOS}
    for seed in range(2000):
        case,_,expected=scenario(seed);counts[case][signature(expected)]+=1
    Path("tests/fixtures/custody_quantity_primitive_counts.json").write_text(json.dumps(counts,sort_keys=True,indent=2)+"\n")
