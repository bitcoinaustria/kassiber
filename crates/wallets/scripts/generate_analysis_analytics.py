from __future__ import annotations

import copy
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from kassiber.core.chain_analysis.analytics import analyze_index
from kassiber.core.chain_analysis.index import AnalysisIndex, observer_index

DEST = ROOT / "crates/wallets/tests/data/analysis_analytics.json"


def evaluate(case, snapshots):
    index = observer_index(AnalysisIndex(**snapshots[case["snapshot"]]), case.get("query", {}).get("observer", "public"))
    selected = set(case.get("selected", index.nodes))
    result = analyze_index(index, case.get("query", {}), selected)
    if case["group"] == "full":
        return result
    if case["group"] == "attribution":
        return {"exposure":result["exposure"],"coverage":{"stopped_reasons":[r for r in result["coverage"]["stopped_reasons"] if r.startswith("exposure_")],"label_count":result["coverage"]["label_count"]}}
    if case["group"] == "patterns":
        return {"patterns":result["patterns"],"coverage":{"stopped_reasons":[r for r in result["coverage"]["stopped_reasons"] if r.startswith("pattern_")],"pattern_count":len(result["patterns"])}}
    coverage = result["coverage"]
    coverage.pop("pattern_count")
    coverage["stopped_reasons"] = [r for r in coverage["stopped_reasons"] if r in {"hypothesis_budget", "label_analysis_budget"}]
    return {key: result[key] for key in ["rule_version", "findings", "clusters", "hypothesis_edges", "coverage"]}


def vectors():
    captured = json.loads((DEST.parent / "analysis_traversal.json").read_text())["snapshots"]
    snapshots = {k: captured[k] for k in ["diamond", "large", "stale", "conflicting", "empty"]}
    cases = []
    def add(name, snapshot="diamond", **kw):
        case = {"name": name, "group": "clustering", "snapshot": snapshot, **kw}
        case["expected"] = evaluate(case, snapshots)
        cases.append(case)
    for name in snapshots:
        for observer in ["public", "owner", "disclosed", "invalid"]:
            for enabled in [True, False]:
                add(f"{name}-{observer}-{enabled}", snapshot=name, query={"observer": observer, "include_hypotheses": enabled})
    base = snapshots["diamond"]
    outputs = [k for k, n in base["nodes"].items() if n["kind"] == "output"]
    transactions = list(base["transaction_facts"])
    for name, change in [
        ("collaboration", lambda v: [f.update(collaboration={"kind": "coinjoin", "evidence_level": "reviewed", "source": "local"}) for f in v["transaction_facts"].values()]),
        ("private-change", lambda v: ([f.update(branch_role="change") for f in v["output_facts"].values()], [n.update(wallet_ids=["private-owner"]) for n in v["nodes"].values() if n["kind"] == "output"])),
        ("op-return", lambda v: [f.update(script="6a00") for f in v["output_facts"].values()]),
        ("unknown-script", lambda v: [f.update(script=None) for f in v["output_facts"].values()]),
        ("incomplete", lambda v: [f.update(complete=False) for f in v["transaction_facts"].values()]),
        ("unknown-amount", lambda v: [r.update(amount_msat=None) for f in v["transaction_facts"].values() for r in f["inputs"]]),
        ("noncanonical-amount", lambda v: [r.update(amount_msat="01") for f in v["transaction_facts"].values() for r in f["outputs"]]),
        ("wrong-domain", lambda v: [n.update(network="unsupported") for n in v["nodes"].values()]),
        ("liquid", lambda v: [n.update(chain="liquid",network="liquidv1") for n in v["nodes"].values()]),
        ("edge-conflict", lambda v: [e.update(status="conflicting") for e in v["edges"].values() if e["kind"] == "spends"]),
    ]:
        changed=copy.deepcopy(base)
        change(changed)
        snapshots[name]=changed
        for observer in ["public", "owner", "disclosed"]:
            add(f"{name}-{observer}", snapshot=name, query={"observer": observer,"include_hypotheses":True})
    for defining in [False, True, 1, 1.0]:
        for visibility in ["public", "private"]:
            for dataset in [False, True]:
                changed=copy.deepcopy(base)
                changed["labels"]=[{"id": f"label-{i}", "revision": i, "subject": ident, "chain": "bitcoin", "network": "main", "source": "test", "label": "entity 雪", "confidence": "low", "cluster_defining": defining, "visibility":visibility, **({"dataset_id":"pack"} if dataset else {})} for i,ident in enumerate(outputs[:4])]
                name=f"labels-{defining}-{type(defining).__name__}-{visibility}-{dataset}"
                snapshots[name]=changed
                for observer in ["public", "owner", "disclosed"]:
                    add(f"{name}-{observer}",snapshot=name,query={"observer":observer,"include_hypotheses":True})
    for limit in [1, 500, 2001]:
        changed=copy.deepcopy(base)
        changed["labels"]=[{"id":f"label-{i}","node_ids":outputs[:2],"chain":"bitcoin","network":"main","source":"test","label":"same entity","cluster_defining":True,"revision":i} for i in range(limit)]
        name=f"label-budget-{limit}"
        snapshots[name]=changed
        add(name,snapshot=name,query={"observer":"owner","include_hypotheses":True})
    for selected in [[],outputs[:2],transactions,outputs[:3]+transactions[:2]]:
        add(f"selection-{len(cases)}",selected=selected,query={"include_hypotheses":True})
    for selected in [[],list(base["edges"])[:2],list(base["edges"])[::2]]:
        add(f"edge-selection-{len(cases)}",query={"include_hypotheses":True,"_selected_edge_ids":selected})
    cases.extend(pattern_vectors(snapshots))
    cases.extend(attribution_vectors(snapshots))
    cases.extend(composition_vectors(snapshots))
    return {"snapshots":snapshots,"cases":cases}


def capture_spec(spec):
    from generate_analysis_index import database, snapshot
    from kassiber.core.chain_analysis.index import build_index
    conn = database()
    conn.execute("DELETE FROM transactions")
    def script(number, vout):
        return "0014" + f"{number * 1000 + vout:040x}"
    for number, (parents, amounts) in spec.items():
        raw = {"txid": f"{number:064x}", "chain":"bitcoin", "network":"main", "version":2, "locktime":0,
               "vin":[{"txid":f"{parent:064x}","vout":vout,"prevout":{"value":spec[parent][1][vout],"scriptpubkey":script(parent,vout)}} for parent,vout in parents] or [{"coinbase":"01"}],
               "vout":[{"value":value,"scriptpubkey":script(number,vout)} for vout,value in enumerate(amounts)]}
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,created_at,raw_json) VALUES(?,'ws','p','cold',?,?,'BTC','inbound',1000,0,'2026-01-01T00:00:00Z','fixed',?)",(f"pattern-{number}",f"pattern-{number}",raw["txid"],json.dumps(raw)))
    captured = snapshot(build_index(conn,"p"))
    conn.close()
    return captured


def pattern_vectors(snapshots):
    snapshots["peel"] = capture_spec({1:([],[1_000_000]),**{n:([(n-1,0)],[1_000_000-n*10_000,9000]) for n in range(2,8)}})
    spec={1:([],[100000]*20)}
    spec.update({n:([(1,n-2)],[4999]*20) for n in range(2,22)})
    spec.update({n:([(p,n-22) for p in range(2,22)],[4998]*20) for n in range(22,42)})
    spec.update({n:([(p,n-42) for p in range(22,42)],[49979]*2) for n in range(42,62)})
    snapshots["dense"] = capture_spec(spec)
    original=copy.deepcopy(snapshots["large"])
    def transformed(value):
        if isinstance(value,str): return value.replace("bitcoin:main:","bitcoin:regtest:")
        if isinstance(value,list): return [transformed(v) for v in value]
        if isinstance(value,dict): return {transformed(k):transformed(v) for k,v in value.items()}
        return value
    other=transformed(original)
    for node in other["nodes"].values(): node["network"]="regtest"
    for name in ["nodes","edges","outgoing","incoming","subjects","transaction_facts","output_facts"]:
        for key,value in other[name].items():
            if name=="subjects" and key in original[name]: original[name][key].extend(value)
            else: original[name][key]=value
    original["profile_seeds"].extend(other["profile_seeds"])
    snapshots["transaction-budget"] = original
    postmix=copy.deepcopy(snapshots["diamond"])
    first=next(iter(postmix["transaction_facts"]))
    postmix["transaction_facts"][first]["collaboration"]={"kind":"coinjoin","source":"equal_output_coinjoin","evidence_level":"heuristic"}
    snapshots["postmix"]=postmix
    cases=[]
    for name in ["diamond","peel","postmix","dense","transaction-budget","stale","conflicting","edge-conflict","liquid","empty"]:
        for depth in [1,6,12]:
            for enabled in [True,False]:
                for observer in ["public","owner"]:
                    case={"name":f"patterns-{name}-{depth}-{enabled}-{observer}","group":"patterns","snapshot":name,"query":{"depth":depth,"include_hypotheses":enabled,"observer":observer}}
                    case["expected"]=evaluate(case,snapshots)
                    cases.append(case)
    return cases


def attribution_vectors(snapshots):
    cases=[]
    first="bitcoin:main:tx:"+f"{1:064x}"
    last="bitcoin:main:tx:"+f"{7:064x}"
    def add(name,snapshot,query):
        case={"name":name,"group":"attribution","snapshot":snapshot,"query":query}
        case["expected"]=evaluate(case,snapshots); cases.append(case)
    for name in ["labels-True-bool-public-True","labels-True-bool-private-True","labels-True-bool-public-False","label-budget-500"]:
        for subject in [first,last,None]:
            for depth in [1,6,12]:
                for observer in ["public","owner"]:
                    for include in [True,False]:
                        add(f"attribution-{len(cases)}",name,{"subject":subject,"depth":depth,"observer":observer,"include_relations":include})
    root="liquid:liquidv1:tx:"+"f"*64
    node={"id":root,"kind":"transaction","chain":"liquid","network":"liquidv1","status":"observed","wallet_ids":[],"evidence":[]}
    dense=copy.deepcopy(snapshots["dense"])
    dense["nodes"][root]=node
    dense["subjects"][root]=[root]
    target=next(k for k,n in dense["nodes"].items() if n["kind"]=="output")
    dense["labels"]=[{"id":f"budget-{i}","node_ids":[target],"chain":"bitcoin","network":"main","label":"Sourced entity","source":"fixture","dataset_id":"public-pack","visibility":"public","cluster_defining":False} for i in range(500)]
    snapshots["dense-labels"]=dense
    for depth in [1,12]:
        for observer in ["public","owner"]:
            add(f"attribution-dense-{depth}-{observer}","dense-labels",{"subject":root,"depth":depth,"observer":observer})
    for status in ["observed","stale"]:
        changed=copy.deepcopy(snapshots["labels-True-bool-public-True"])
        changed["nodes"][root]=node
        changed["subjects"][root]=[root]
        source=changed["labels"][0]["subject"]
        changed["edges"]["custody-exposure"]={"id":"custody-exposure","source":source,"target":root,"kind":"custody","status":status,"evidence":[{"source":"local_custody_review","reference":"relation"}]}
        name="attribution-custody-"+status
        snapshots[name]=changed
        for include in [True,False]:
            for observer in ["public","owner"]:
                add(f"{name}-{include}-{observer}",name,{"subject":root,"depth":6,"observer":observer,"include_relations":include})
    return cases


def composition_vectors(snapshots):
    cases = []
    for name in snapshots:
        snapshot = snapshots[name]
        txids = list(snapshot['transaction_facts'])
        for observer in ['public','owner','disclosed']:
            for entropy in [False,True]:
                query = {'observer':observer,'include_hypotheses':True,'entropy':entropy}
                if txids:
                    query['subject'] = txids[-1]
                c = {'name':f'full-{name}-{observer}-{entropy}','group':'full','snapshot':name,'query':query}
                c['expected'] = evaluate(c,snapshots)
                cases.append(c)
    for query in [{'mode':'entropy'},{'action':'entropy'},{'entropy':True,'subject':'missing'},{'entropy':True,'_subject_node_ids':[]},{'entropy':1},{'entropy':True,'entropy_max_states':1},{'entropy':True,'_selected_edge_ids':[]},{'entropy':True,'subject':'missing','observer':'invalid'}]:
        c = {'name':f'full-extra-{len(cases)}','group':'full','snapshot':'diamond','query':query}
        c['expected'] = evaluate(c,snapshots)
        cases.append(c)
    return cases


def main():
    result=vectors()
    if "--write" in sys.argv:
        DEST.write_text("{\n\"snapshots\":{\n" + ",\n".join(json.dumps(k) + ":" + json.dumps(v,ensure_ascii=True,separators=(",",":")) for k,v in result["snapshots"].items()) + "\n},\n\"cases\":[\n" + ",\n".join(json.dumps(c,ensure_ascii=True,separators=(",",":")) for c in result["cases"]) + "\n]\n}\n")
    elif json.loads(DEST.read_text()) != result:
        raise SystemExit("analysis analytics vectors drifted")
    print(f"{len(result['cases'])} analysis analytics vectors")


if __name__ == "__main__":
    main()
