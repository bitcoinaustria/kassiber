"""Independent CPython quantity primitive vectors; never generated from Rust."""
from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from kassiber.core import custody_quantity as q
from kassiber.core.custody_allocations import allocate_msat_fifo
from kassiber.core.custody_quantity_runtime import compare_wallet_balances
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_quantity_primitives.json"


def outcome(call):
    try:
        return {"expected": to_jsonable(call(), redact_config=False)}
    except Exception as exc:
        return {"error": getattr(exc, "code", type(exc).__name__)}


def construct_claim(v):
    fields={k:deepcopy(val) for k,val in v.items() if k!="$type"}
    for side in ("source","target"):
        if fields.get(side) is not None:
            fields[side]=q.QuantitySlice(**{k:val for k,val in fields[side].items() if k!="$type"})
    if isinstance(fields["priority"],dict):
        fields["priority"]=q.ClaimPriority(fields["priority"]["value"])
    return q.QuantityClaim(**fields)


def project_cases():
    from tests import test_custody_quantity as pins
    cases = []
    original = q.project_quantities
    from tests import test_custody_quantity_runtime as runtime_pins
    from kassiber.core import custody_quantity_runtime as runtime
    for cls, module in ((pins.CustodyQuantityTests, pins), (runtime_pins.CustodyQuantityRuntimeTests, runtime)):
      for name in unittest.defaultTestLoader.getTestCaseNames(cls):
        def capture(observations, claims):
            claims = tuple(claims)
            case = dict(case=name.removeprefix("test_") + f"_{sum(c['case'].startswith(name.removeprefix('test_') + '_') for c in cases)}",
                        observations=to_jsonable(observations, redact_config=False),
                        claims=to_jsonable(claims, redact_config=False))
            case.update(outcome(lambda: original(observations, claims)))
            cases.append(case)
            return original(observations, claims)
        with patch.object(module, "project_quantities", capture):
            getattr(cls(name), name)()
    source = pins._observation("source", "old", "outbound", 100)
    target = pins._observation("target", "new", "inbound", 100)
    def add(name, observations, claims=()):
        cases.append(dict(case=name, observations=to_jsonable(observations, redact_config=False),
                          claims=to_jsonable(claims, redact_config=False),
                          **outcome(lambda: original(observations, claims))))
    def claim(name="native", **fields):
        return q.QuantityClaim(claim_id=name, source=q.QuantitySlice(source.quantity_hash, 0, 100),
                               state=q.INTERNAL_VERIFIED, priority=q.ClaimPriority.EXACT_NATIVE_EVENT,
                               target=q.QuantitySlice(target.quantity_hash, 0, 100), reason="native", **fields)
    base = claim()
    for name, changed in {
        "source_missing": replace(base, source=q.QuantitySlice("f" * 64, 0, 100)),
        "source_inbound": replace(base, source=q.QuantitySlice(target.quantity_hash, 0, 100)),
        "source_exceeds": replace(base, source=q.QuantitySlice(source.quantity_hash, 1, 101)),
        "target_missing": replace(base, target=q.QuantitySlice("f" * 64, 0, 100)),
        "target_outbound": replace(base, target=q.QuantitySlice(source.quantity_hash, 0, 100)),
        "target_exceeds": replace(base, target=q.QuantitySlice(target.quantity_hash, 1, 101)),
    }.items():
        add(name, [source, target], [changed])
    add("duplicate_observation", [source, source])
    add("duplicate_claim", [source, target], [base, base])
    add("priority_mismatch", [source, target], [replace(base, atomic_bundle_id="bundle"),
        q.QuantityClaim("hold", q.QuantitySlice(source.quantity_hash, 0, 100), q.CUSTODY_SUSPENSE,
                        q.ClaimPriority.ACCOUNTING_CONVENTION, "hold", atomic_bundle_id="bundle")])
    add("multiple_malformed_source_bundles",[source,target],[replace(base,target=q.QuantitySlice("f"*64,0,100)),
        replace(base,claim_id="reviewed",state=q.INTERNAL_REVIEWED,priority=q.ClaimPriority.REVIEWED_COMPONENT,target=q.QuantitySlice("e"*64,0,100))])
    for field, value in (("transaction_id", ""), ("anchor_transaction_id", ""), ("wallet_id", ""),
                         ("quantity_hash", "short"), ("evidence_detail_hash", "z" * 64),
                         ("direction", "other"), ("asset", ""), ("amount_msat", True),
                         ("amount_msat", -1), ("fee_msat", True), ("fee_msat", -1),
                         ("amount_msat", 0), ("fee_msat", 101)):
        ob = replace(source, **{field:value}, **({"amount_includes_fee":True} if field == "fee_msat" and value == 101 else {}))
        add(f"invalid_observation_{field}_{value}", [ob])
    add("invalid_inbound_inclusive", [replace(target, amount_includes_fee=True)])
    add("empty", [])
    for asset in ("LBTC", "USDT"):
        for network in ("liquidv1", "liquidtestnet", "elementsregtest"):
            for cross in (False, True):
                ob = replace(target, asset=asset, event_key=replace(target.event_key, chain="liquid", network=network))
                add(f"domain_{asset}_{network}_{cross}", [source, ob], [replace(base, allow_cross_rail=cross)])
    # Above machine-size amounts and one-msat disjoint boundaries.
    for n in (1, 999, 1001, 2**127 + 1, 10**100 + 1):
        out = replace(source, amount_msat=n, fee_msat=n//3)
        incoming = replace(target, amount_msat=n)
        add(f"exact_{n}", [out, incoming], [replace(base,
            source=q.QuantitySlice(source.quantity_hash, 0, n),
            target=q.QuantitySlice(target.quantity_hash, 0, n))])
    # Constructor guards are part of the primitive admission contract. These
    # malformed objects can never come from an admitted interpreter stage.
    construct=construct_claim
    base_json=to_jsonable(base,redact_config=False)
    invalid_fields = (("claim_id",""),("reason",""),("priority",20),("priority",{"value":30}),
        ("state","external_presumed"),("state",q.INTERNAL_REVIEWED),
        ("target",None),("atomic_bundle_id","\x1c\u3000"),
        ("destination_kind","other"),("destination_kind","external"),("destination_kind","retained_custody"),
        ("external_economic_subtype","other"),("external_economic_subtype","gift"))
    for index,(field,value) in enumerate(invalid_fields):
        raw=deepcopy(base_json);raw[field]=value
        cases.append(dict(case=f"invalid_claim_{index}",observations=to_jsonable([source,target],redact_config=False),claims=[raw],**outcome(lambda:construct(raw))))
    for name,changes in (("forbidden_target",dict(state=q.CUSTODY_SUSPENSE)),
                          ("targetless_cross_rail",dict(state=q.EXTERNAL_CONFIRMED,target=None,allow_cross_rail=True)),
                          ("external_convention",dict(state=q.EXTERNAL_CONFIRMED,target=None,priority={"value":40})),
                          ("unsupported_targetless",dict(state="unsupported",target=None))):
        raw=deepcopy(base_json);raw.update(changes)
        cases.append(dict(case=name,observations=to_jsonable([source,target],redact_config=False),claims=[raw],**outcome(lambda:construct(raw))))
    for side in ("source","target"):
        for field,value in (("observation_hash",""),("start_msat",True),("start_msat",-1),("end_msat",0),("end_msat",101)):
            raw=deepcopy(base_json);raw[side][field]=value
            cases.append(dict(case=f"invalid_slice_{side}_{field}_{value}",observations=to_jsonable([source,target],redact_config=False),claims=[raw],**outcome(lambda:construct(raw))))
        raw=deepcopy(base_json);raw[side].update(start_msat=-1,end_msat=99)
        cases.append(dict(case=f"negative_slice_preserves_amount_{side}",observations=to_jsonable([source,target],redact_config=False),claims=[raw],**outcome(lambda:construct(raw))))
    raw=deepcopy(base_json)
    for side in ("source","target"):raw[side]["end_msat"]=0
    cases.append(dict(case="empty_slices_preserve_amount",observations=to_jsonable([source,target],redact_config=False),claims=[raw],**outcome(lambda:construct(raw))))
    for state,priority in ((q.EXTERNAL_CONFIRMED,q.ClaimPriority.ACCOUNTING_CONVENTION),
                           (q.INTERNAL_VERIFIED,q.ClaimPriority.REVIEWED_COMPONENT),
                           (q.INTERNAL_REVIEWED,q.ClaimPriority.EXACT_NATIVE_EVENT)):
        raw=deepcopy(base_json);raw.update(state=state,priority=to_jsonable(priority,redact_config=False))
        cases.append(dict(case=f"priority_{state}",observations=to_jsonable([source,target],redact_config=False),claims=[raw],**outcome(lambda:construct(raw))))
    for field in ("component_id","transfer_kind","transfer_policy"):
        add(f"semantic_{field}",[source,target],[base,replace(base,claim_id="other",**{field:"reviewed"})])
    add("semantic_target_offsets",[source,replace(target,amount_msat=101)],[base,replace(base,claim_id="other",target=q.QuantitySlice(target.quantity_hash,1,101))])
    add("equal_semantics_different_reason",[source,target],[base,replace(base,claim_id="other",reason="different")])
    for subtype in ("payment","disposal","gift","lost"):
        external=q.QuantityClaim("external",q.QuantitySlice(source.quantity_hash,0,100),q.EXTERNAL_CONFIRMED,
            q.ClaimPriority.REVIEWED_COMPONENT,"review",destination_kind="external",external_economic_subtype=subtype,
            transfer_kind="swap",transfer_policy="taxable",component_id="review")
        add(f"subtype_{subtype}",[source],[external])
        add(f"semantic_subtype_{subtype}",[source],[external,replace(external,claim_id="other",external_economic_subtype=None)])
    external=q.QuantityClaim("external",q.QuantitySlice(source.quantity_hash,0,100),q.EXTERNAL_CONFIRMED,
        q.ClaimPriority.REVIEWED_COMPONENT,"review",destination_kind="external")
    add("semantic_destination",[source],[external,replace(external,claim_id="fee",destination_kind="fee")])
    for value in ("A"*64,"_"+"0"*63,"0"*63+"_","+"+"0"*63,"0x_"+"0"*61,"١"*64,"٠x_"+"١"*61,"0"*62+"__","0"*62+"é0"," "*63+"0", "\x1c"+"0"*63):
        add(f"hash_spelling_{len(cases)}",[replace(source,quantity_hash=value)])
    from tests.custody_quantity_generator import SCENARIOS,scenario
    for seed in range(len(SCENARIOS)):
        case,value,expected=scenario(seed)
        cases.append(dict(case=f"generated_{case}",**value,expected=expected))
    return cases


def allocation_cases():
    cases=[]
    def add(name,sources,sinks,**amount):
        try:
            result=allocate_msat_fifo(sources,sinks,**amount)
            result={"expected":to_jsonable(result,redact_config=False)}
        except Exception as exc:
            result=dict(error=exc.code,message=exc.message,details=exc.details)
        cases.append(dict(case=name,sources=sources,sinks=sinks,**amount,**result))
    for n in (1,999,1000,1001,2**127+1,10**100+1):
        for requested in (0,1,n,2*n,None):
            add(f"nm_{n}_{requested}", [["zero",0],["a",n],["b",n]], [["x",n],["zero",0],["y",n]],amount_msat=requested)
    for value in (None,1,True,"bad",{},[1],[["a"]],[["a",1,2]],[[None,1]],[[False,1]],[[0,1]],[["a",True]],[["a",1.0]],[["a","1"]],[["a",-1]],[["a",1],["a",2]],[[1,1],["1",1]]):
        add(f"sources_{len(cases)}",value,[["sink",1]])
        add(f"sinks_{len(cases)}",[["source",1]],value)
    for value in (False,True,-1,1.0,"1",[],{}):
        add(f"amount_{len(cases)}",[["source",10]],[["sink",10]],amount_msat=value)
    add("insufficient_source",[["a",1]],[["b",2]])
    add("insufficient_sink",[["a",2]],[["b",1]],amount_msat=2)
    add("empty",[],[])
    add("empty_requested",[],[],amount_msat=1)
    add("id_coercion",[[True,1],[7,2]],[[["array"],3]])
    add("consecutive_sinks",[["source",3]],[["first",1],["second",1],["third",1]])
    add("default_sink_total",[["source",10]],[["sink",3]])
    add("both_invalid_sequences",None,False,amount_msat=-1)
    add("both_invalid_buckets",[["source","bad"]],[["bad"]],amount_msat=False)
    return cases


def balance_cases():
    from types import SimpleNamespace
    from tests.test_custody_quantity import _observation
    observations=[_observation("out", "old", "outbound", 1001, fee_msat=1),
                  _observation("in", "new", "inbound", 999),
                  _observation("other", "old", "inbound", 1)]
    cases=[]
    for obs in ([],observations,observations[:1]):
        projection=q.project_quantities(obs,[])
        for current in ({},{("old","BTC"):-1001,("new","BTC"):999},
                        {("old","BTC"):10**100,("new","BTC"):-1,("absent","BTC"):1},
                        {("old","BTC"):"-1001",("new","BTC"):True}):
            for known in ({},{"out":"channel","other":"channel"},{"out":"first","other":"second"}):
                cases.append(dict(projection=to_jsonable(projection,redact_config=False),
                    current=[[wallet,asset,n] for (wallet,asset),n in current.items()],known=known,
                    **outcome(lambda: compare_wallet_balances(SimpleNamespace(projection=projection),current,known_non_event_reasons=known))))
    return cases


def vectors():
    project=project_cases()
    claims={json.dumps(claim,sort_keys=True):claim for case in project for claim in case["claims"]}
    admission=[]
    for key,claim in sorted(claims.items()):
        result=outcome(lambda:construct_claim(claim))
        admission.append(dict(claim=claim,error=result.get("error")))
    return dict(project=project,claim_admission=admission,allocation=allocation_cases(),balances=balance_cases())


if __name__=="__main__":
    import sys
    data=vectors()
    if "--write" in sys.argv:
        PATH.write_text(json.dumps(data,sort_keys=True,separators=(",",":"))+"\n")
    else:
        assert json.loads(PATH.read_text())==data
    print({k:len(v) for k,v in data.items()})
