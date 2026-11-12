"""Python-owned branch witnesses for the isolated ownership producer.

The raw request stays schema 1. Direct cases deliberately include values beyond
snapshot admission to pin the producer's conversion order and error families.
"""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core.custody_snapshot import decode_owned_index
from kassiber.core.ownership_transfers import derive_ownership_transfers
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_graph_generator import graph_snapshot, GRAPH_SCENARIOS
from tests.custody_graph_vectors import match
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_ownership_cases.json"


def books():
    for case in GRAPH_SCENARIOS:
        yield case, graph_snapshot(0, case=case), []
    cases = (
        "duplicate_wallet_ref", "wrong_outpoint_only", "wrong_script_only", "historical_multi_input", "small_tolerance_floor", "no_index_alias", "no_index_different_scope", "synthetic_bad_source", "zero_liquid_graphless", "small_zero_receipt", "small_negative_receipt", "liquid_ambiguous_other_asset", "no_index", "no_index_liquid_shortfall", "no_index_liquid_exact", "no_index_handled", "no_index_no_scope", "no_index_other_asset", "no_index_other_wallet",
        "graphless_liquid_shortfall", "graphless_liquid_exact", "graphless_bitcoin", "graphless_liquid_handled", "handled_source", "synthetic_source", "zero_source", "negative_source", "bad_source", "bad_fee", "type_source",
        "source_missing", "source_present", "source_missing_handled", "source_missing_untrusted", "source_self", "source_zero_delta", "source_negative_delta", "source_unknown_delta",
        "synthetic_inbound", "reuse", "reuse_paired", "reuse_other_asset", "reuse_zero", "reuse_negative", "reuse_bad", "reuse_two_exact", "reuse_near", "reuse_far", "reuse_different_tx", "reuse_unscoped", "reuse_wrong_network", "reuse_no_ref", "reuse_ref_not_needed",
        "duplicate", "duplicate_handled", "distinct_economic", "duplicate_fee_error_order", "multi_input", "foreign_input", "shared_input", "empty_input", "historical_fallback", "historical_absent", "historical_ambiguous", "historical_no_outpoint", "wrong_input_network", "wrong_txid_network", "historical_script", "historical_scoped_absent",
        "ordinary_foreign_inputs", "ordinary", "change", "shared_change", "ambiguous", "ambiguous_external", "ambiguous_external_unknown", "unknown_owned_value", "unknown_change_value", "unknown_external_value", "unknown_input_value", "asset_unknown_input", "asset_other_input", "asset_unknown_output", "asset_other_output", "asset_other_external", "asset_other_change", "asset_display", "asset_display_other", "asset_missing", "asset_nonfinite", "asset_literal_tag", "conflicts",
        "liquid_conservation", "liquid_change", "liquid_change_mismatch", "liquid_zero_inputs", "liquid_multifees", "liquid_wrongfee", "liquid_unknownfee", "liquid_negativefee", "liquid_fee_display", "liquid_fee_fallback", "liquid_fee_other_unknown", "liquid_explicit_row",
        "over_amount", "inclusive_exact", "inclusive_unknown", "inclusive_too_big", "inclusive_zero_gap", "implicit", "explicit_fee", "residual", "fee_cap", "fee_negative", "fee_zero", "nonpositive_leg", "negative_leg", "two_destinations", "destination_order", "destination_tie", "destination_aggregate", "destination_missing_ref", "canonical_id", "bad_identity", "unbounded", "unbounded_index",
        "pricing", "pricing_exact", "pricing_existing", "pricing_error", "rollback", "consumed", "rollback_zero_leg", "source_order", "scope_conflict_before_script", "incomplete_before_script", "fee_before_output", "tolerance_boundary", "tolerance_outside", "tolerance_percent", "tolerance_percent_outside",
    )
    for case in cases:
        liquid = case.startswith(("liquid_", "asset_", "zero_liquid", "no_index_alias", "no_index_different_scope")) or case in {"no_index_liquid_shortfall", "no_index_liquid_exact", "no_index_handled", "no_index_no_scope", "no_index_other_asset", "no_index_other_wallet", "graphless_liquid_shortfall", "graphless_liquid_exact", "graphless_liquid_handled", "unknown_change_value", "unknown_input_value"}
        book = graph_snapshot(0, case="liquid_ownership" if liquid else "bitcoin_ownership")
        source = book["transactions"][0]
        original_raw = source["raw_json"]
        raw = json.loads(original_raw)
        raw.pop("raw_hex", None)
        point = f"{raw['vin'][0]['txid']}:0"
        script_a = raw["vin"][0]["prevout"]["scriptpubkey"]
        script_c = raw["vout"][0]["scriptpubkey"]
        index = book["owned_index"]
        handled = []
        def inbound(**kwargs):
            row = dict(source, id="receipt", direction="inbound", wallet_id="wallet-c", fee=0, amount=raw["vout"][0].get("value_sats", 0)*1000)
            row.update(kwargs)
            book["transactions"].append(row)
            return row
        def other_output(n=2, wallet="wallet-b", sats=100, **kwargs):
            script = next(s for s,ms in index["by_script"].items() if ms[0]["wallet_id"]==wallet)
            out = dict(raw["vout"][0], n=n, scriptpubkey=script, value_sats=sats)
            out.update(kwargs)
            raw["vout"].append(out)
            return out
        if case.startswith(("no_index_", "graphless_")):
            inbound(amount=source["amount"] if case.endswith("exact") else source["amount"]-1000)
            raw.pop("vin"); raw.pop("vout")
            if case.startswith("no_index_"): book["owned_index"] = None
            if case in {"no_index_handled", "graphless_liquid_handled"}: handled = [source["id"]]
            if case == "no_index_no_scope": source["external_id"] = "provider"; raw.pop("txid")
            if case == "no_index_alias":book["transactions"][1]["asset"]="L-BTC"
            if case == "no_index_different_scope":
                row=book["transactions"][1];row["external_id"]="de"*32;row["raw_json"]=json.dumps(dict(raw,txid="de"*32))
            if case == "no_index_other_asset": book["transactions"][1]["asset"] = "OTHER"
            if case == "no_index_other_wallet": book["transactions"][1]["wallet_id"] = source["wallet_id"]
        if case == "duplicate_wallet_ref":
            original=next(w for w in book["wallets"] if w["id"]=="wallet-c")
            book["wallets"].append(dict(original,label="Latest destination",account_code="LAST"))
        if case in {"wrong_outpoint_only","wrong_script_only"}:
            raw["network"]="test";source["config_json"]='{"chain":"bitcoin","network":"test"}'
            if case=="wrong_outpoint_only":
                for script in (script_a,script_c):index["by_script"][script][0]["network"]="test"
                index["txid_wallets_by_scope"][0]["key"][1]="test"
            else:
                index["by_outpoint"]={};index["by_scoped_outpoint"]=[];index["txid_wallets"]={};index["txid_wallets_by_scope"]=[]
        if case == "small_tolerance_floor":
            raw["vout"][0]["value_sats"]=1;source["amount"]=1000
            inbound(amount=2501000,external_id="provider",raw_json="{}")
        if case == "historical_multi_input":
            raw["vin"].append(dict(txid="ab"*32,vout=0,prevout=dict(scriptpubkey="abcd",value_sats=1)))
        if case == "synthetic_bad_source": source.update(id="owned-derive:bad",amount="bad")
        if case == "zero_liquid_graphless":
            source["amount"]=0;inbound(amount=1000);raw.pop("vin");raw.pop("vout")
        if case in {"small_zero_receipt","small_negative_receipt"}:
            raw["vout"][0]["value_sats"]=1;source["amount"]=1000
            inbound(amount=0 if case=="small_zero_receipt" else -1)
        if case == "liquid_ambiguous_other_asset":
            index["by_script"][script_c].append(match("wallet-b",chain="liquid",network="elementsregtest"))
            raw["vout"].append(dict(raw["vout"][0],n=2,scriptpubkey="abcd",asset_id="ee"*32,value_sats=None))
        if case == "no_index": book["owned_index"] = None
        if case == "handled_source": handled = [source["id"]]
        if case == "synthetic_source": source["id"] = "owned-derive:source"
        if case == "zero_source": source["amount"] = 0
        if case == "negative_source": source["amount"] = -1
        if case == "bad_source": source["amount"] = "bad"
        if case == "bad_fee": source["fee"] = "bad"
        if case == "type_source": source["amount"] = [1]
        if case.startswith("source_") and case not in {"source_order"}:
            row = inbound()
            if case in {"source_zero_delta", "source_negative_delta", "source_unknown_delta"}:
                other_output(n=2, wallet="wallet-a", sats=raw["vin"][0]["prevout"]["value_sats"]+(1 if case=="source_negative_delta" else 0))
                # Keep total event conservation while adjusting the source delta.
                raw["vin"].append(dict(txid="ab"*32,vout=0,prevout=dict(scriptpubkey=next(s for s,ms in index["by_script"].items() if ms[0]["wallet_id"]=="wallet-b"),value_sats=raw["vout"][0]["value_sats"]+(1 if case=="source_negative_delta" else 0))))
                if case=="source_unknown_delta": raw["vin"][0]["prevout"].pop("value_sats")
            if case != "source_present": book["transactions"].remove(source)
            if case == "source_missing_handled": handled = [row["id"]]
            if case == "source_missing_untrusted": row["observation_authority_version"] = None
            if case == "source_self": row["wallet_id"] = "wallet-a"
        if case.startswith("reuse") or case in {"synthetic_inbound", "consumed", "tolerance_boundary", "tolerance_outside", "tolerance_percent", "tolerance_percent_outside"}:
            row = inbound()
            if case == "reuse_paired": handled = [row["id"]]
            if case == "reuse_other_asset": row["asset"] = "OTHER"
            if case == "reuse_zero": row["amount"] = 0
            if case == "reuse_negative": row["amount"] = -1
            if case == "reuse_bad": row["amount"] = "bad"
            if case == "reuse_two_exact": inbound(id="receipt-2")
            if case == "reuse_near": row["amount"] -= 1000
            if case == "reuse_far": row["amount"] += 10000000
            if case == "reuse_different_tx": row["external_id"] = "de"*32; row["raw_json"] = json.dumps(dict(raw, txid=row["external_id"]))
            if case == "reuse_unscoped": row["external_id"] = "provider"; row["raw_json"] = "{}"
            if case == "reuse_wrong_network": row["config_json"] = '{"chain":"bitcoin","network":"test"}'; row["raw_json"] = "{}"
            if case == "reuse_no_ref": book["wallets"] = [w for w in book["wallets"] if w["id"] != "wallet-c"]
            if case == "reuse_ref_not_needed": book["wallets"] = []
            if case == "synthetic_inbound": row["id"] = "direct-payout:receipt"
            if case == "consumed": book["transactions"].append(dict(source,id="source-2",amount=source["amount"]+1,fee=source["fee"]-1))
            if case.startswith("tolerance_"):
                if "percent" in case:
                    scale=100000
                    source["amount"]*=scale; source["fee"]*=scale
                    for entry in raw["vin"]: entry["prevout"]["value_sats"]*=scale
                    for out in raw["vout"]: out["value_sats"]*=scale
                leg=raw["vout"][0]["value_sats"]*1000
                row["amount"]=leg+max(leg//100,2500000)+(1 if case.endswith("outside") else 0)
                row["external_id"]="provider"; row["raw_json"]="{}"
        if (case.startswith("duplicate") and case != "duplicate_wallet_ref") or case == "distinct_economic":
            other=dict(source,id="duplicate")
            if case=="duplicate_handled": handled=[other["id"]]
            if case=="distinct_economic": other["fee"]+=1
            book["transactions"].append(other)
            if case=="duplicate_fee_error_order": source["fee"]="bad"; other["fee"]=[1]
        if case in {"multi_input", "foreign_input"}: raw["vin"].append(dict(txid="ab"*32,vout=1,prevout=dict(scriptpubkey="abcd",value_sats=1)))
        if case=="multi_input": index["by_script"]["abcd"]=[match("wallet-a")]
        if case=="shared_input": index["by_scoped_outpoint"][0]["matches"].append(match("wallet-b"))
        if case=="empty_input": raw["vin"]=[]
        if case.startswith("historical_") or case=="wrong_txid_network":
            index["by_outpoint"]={}; index["by_scoped_outpoint"]=[]; index["by_script"].pop(script_a)
            if case=="historical_absent": index["txid_wallets_by_scope"]=[]; index["txid_wallets"]={}
            if case=="historical_ambiguous": index["txid_wallets_by_scope"][0]["matches"].append(["wallet-b","B"])
            if case=="historical_no_outpoint": raw["vin"][0].pop("txid")
            if case=="historical_script": index["by_script"][script_a]=[match("wallet-a")]
            if case in {"wrong_txid_network", "historical_scoped_absent"}: index["txid_wallets_by_scope"]=[]
            if case=="historical_scoped_absent": index["txid_wallets"]={}
        if case=="wrong_input_network":
            source["config_json"]='{"chain":"bitcoin","network":"test"}';raw["network"]="test"
        if case in {"ordinary","ordinary_foreign_inputs"}: index["by_script"].pop(script_c)
        if case=="ordinary_foreign_inputs":raw["vin"].append(dict(txid="ab"*32,vout=0,prevout=dict(scriptpubkey="abcd",value_sats=1)))
        if case in {"change","shared_change"}:
            other_output(wallet="wallet-a",sats=200)
            raw["vin"][0]["prevout"]["value_sats"]+=200
            if case=="shared_change": index["by_script"][script_a].append(match("wallet-b"))
        if case.startswith("ambiguous"):
            index["by_script"][script_c].append(match("wallet-b"))
            if case != "ambiguous": raw["vout"].append(dict(n=5,scriptpubkey="abcd",value_sats=10))
            if case=="ambiguous_external_unknown":
                raw["vout"][-1].pop("value_sats")
                raw["vout"].append(dict(n=6,scriptpubkey="abba",value_sats=10))
        if case in {"unknown_owned_value","unknown_external_value"}: raw["vout"][0].pop("value_sats")
        if case=="unknown_external_value": index["by_script"].pop(script_c)
        if case=="unknown_input_value": raw["vin"][0]["prevout"].pop("value_sats")
        if case=="unknown_change_value": other_output(wallet="wallet-a",sats=1).pop("value_sats")
        if case=="asset_unknown_input": raw["vin"][0]["prevout"].pop("asset_id");raw["vin"][0]["prevout"].pop("asset")
        if case=="asset_other_input": raw["vin"][0]["prevout"]["asset_id"]="ee"*32
        if case=="asset_unknown_output": raw["vout"][0].pop("asset_id");raw["vout"][0].pop("asset")
        if case=="asset_other_output": raw["vout"][0]["asset_id"]="ee"*32
        if case=="asset_other_external": raw["vout"].append(dict(raw["vout"][0],n=2,scriptpubkey="abcd",asset_id="ee"*32))
        if case=="asset_other_change": other_output(wallet="wallet-a",sats=20,asset_id="ee"*32)
        if case in {"asset_display","asset_display_other","asset_missing","asset_nonfinite","asset_literal_tag"}:
            raw["component"].pop("asset_id")
            for entry in raw["vin"]:entry["prevout"].pop("asset_id")
            for output in raw["vout"]:output.pop("asset_id")
            if case=="asset_display_other": raw["vout"][0]["asset"]="OTHER"
            if case=="asset_missing": raw["component"].pop("asset");source["asset"]=""
            if case in {"asset_nonfinite","asset_literal_tag"}:
                asset=float("inf") if case=="asset_nonfinite" else {"$float":"inf"}
                raw["component"]["asset"]=asset;source["asset"]="TOKEN"
                for leg in raw["vin"]:leg["prevout"]["asset"]=asset
                for leg in raw["vout"]:leg["asset"]=asset
        if case=="conflicts": raw["ownership_graph"]=dict(raw,vin=[dict(raw["vin"][0],prevout=dict(raw["vin"][0]["prevout"],value_sats=1))])
        if case=="liquid_conservation": source["amount"]+=1
        if case in {"liquid_change","liquid_change_mismatch"}:
            other_output(n=2,wallet="wallet-a",sats=500)
            raw["vin"][0]["prevout"]["value_sats"]+=500+(1 if case.endswith("mismatch") else 0)
        if case=="liquid_zero_inputs": raw["vin"]=[]
        if case in {"liquid_multifees","liquid_wrongfee","liquid_unknownfee","liquid_negativefee","liquid_fee_display","liquid_fee_fallback","liquid_fee_other_unknown"}:
            source["amount"]+=source["fee"];source["amount_includes_fee"]=True
            fee=next(o for o in raw["vout"] if o.get("role")=="fee")
            if case=="liquid_multifees":raw["vout"].append(dict(fee,n=5))
            if case=="liquid_wrongfee":fee["asset_id"]="ee"*32
            if case=="liquid_unknownfee":fee.pop("value_sats")
            if case=="liquid_negativefee":fee["value_sats"]=-1
            if case=="liquid_fee_display":fee.pop("asset_id")
            if case=="liquid_fee_fallback":raw["component"].pop("asset")
            if case=="liquid_fee_other_unknown":raw["vout"].append(dict(fee,n=5,asset_id="ee"*32,value_sats=None))
        if case=="over_amount":source["amount"]-=source["fee"]+1
        if case.startswith("inclusive_"):
            source["amount_includes_fee"]=True
            if case!="inclusive_zero_gap":source["amount"]+=source["fee"]
            if case=="inclusive_unknown":raw["vin"][0]["prevout"].pop("value_sats")
            if case=="inclusive_too_big":raw["vin"][0]["prevout"]["value_sats"]+=100
        if case=="implicit":source["amount_includes_fee"]=True;source["observation_fee_attribution"]=" IMPLICIT_WALLET_DELTA "
        if case=="residual":source["amount"]+=10000000
        if case=="fee_cap":source["amount"]-=1000
        if case=="fee_negative":source["fee"]=-1000
        if case=="fee_zero":source["fee"]=0
        if case in {"nonpositive_leg","negative_leg"}:raw["vout"][0]["value_sats"]=0 if case=="nonpositive_leg" else -1
        if case in {"two_destinations","destination_order","destination_tie","destination_aggregate","destination_missing_ref","rollback","rollback_zero_leg"}:
            other_output(n=1,wallet="wallet-b",sats=0 if case=="rollback_zero_leg" else 100)
            source["amount"]+=100000
            if case=="destination_order":raw["vout"][0]["n"]=10
            if case=="destination_tie":raw["vout"][0]["n"]=1
            if case=="destination_aggregate":raw["vout"][1]["scriptpubkey"]=script_c
            if case in {"destination_missing_ref","rollback"}:book["wallets"]=[w for w in book["wallets"] if w["id"]!="wallet-b"]
            if case.startswith("rollback"):
                book["wallets"]=[w for w in book["wallets"] if w["id"]!="wallet-c"]
                inbound()
                book["transactions"].append(dict(source,id="later",amount=source["amount"]+1))
        if case=="canonical_id":raw["txid"]="provider"
        if case=="bad_identity":raw["txid"]="de"*32
        if case=="unbounded":
            raw["vin"][0]["prevout"]["value_sats"]+=10**80;raw["vout"][0]["value_sats"]+=10**80;source["amount"]+=10**83
        if case=="unbounded_index":raw["vout"][0]["n"]=10**80
        if case.startswith("pricing"):
            source["fiat_value"]="12.3456"
            if case=="pricing_exact":source["fiat_value_exact"]="42.125"
            if case=="pricing_existing":source["fiat_rate"]="900"
            if case=="pricing_error":source["fiat_value"]="bad"
        if case=="source_order":
            book["transactions"]=[dict(source,id="z",fee="bad"),dict(source,id="a",external_id="de"*32,fee=[1],raw_json=json.dumps(dict(raw,txid="de"*32)))]
        if case in {"scope_conflict_before_script","incomplete_before_script"}:
            raw["vout"][0]["scriptpubkey"]=[1]
            if case=="scope_conflict_before_script":source["config_json"]='{"chain":"bitcoin","network":"test"}';raw["network"]="test"
            else:raw["ownership_graph"]=dict(raw,vin=[dict(raw["vin"][0],prevout=dict(raw["vin"][0]["prevout"],value_sats=1))])
        if case=="fee_before_output":source["fee"]="bad";raw["vout"][0]["scriptpubkey"]=[1]
        for row in book["transactions"]:
            if row["raw_json"]==original_raw:row["raw_json"]=json.dumps(raw)
            try:
                row["observation_graph_hash"]=canonical_graph_hash(row["raw_json"])
                row["observation_quantity_hash"]=canonical_observed_quantity_hash(row)
            except (TypeError,ValueError,ArithmeticError):pass
        yield case,book,handled
    for prefix in ("recorded-fanout:", "cross-split:", "direct-payout:", "multi-consol:"):
        book=graph_snapshot(0,case="bitcoin_ownership")
        book["transactions"][0].update(id=prefix+"source",amount="bad")
        yield "synthetic_bad_"+prefix[:-1],book,[]
    book=graph_snapshot(0,case="liquid_ownership")
    source=book["transactions"][0]
    raw=json.loads(source["raw_json"]);raw.pop("vin");raw.pop("vout")
    source["raw_json"]=json.dumps(raw);source["wallet_label"]=""
    book["transactions"].append(dict(source,id="receipt",wallet_id="wallet-c",direction="inbound",amount=source["amount"]-1000))
    book["owned_index"]=None
    yield "block_wallet_fallback",book,[]
    # Declining a later destination preserves the original rows and diagnostics
    # when another source encounters the same candidate population.
    book = graph_snapshot(0, case="bitcoin_ownership")
    first = book["transactions"][0]
    raw = json.loads(first["raw_json"])
    script_b = next(s for s,ms in book["owned_index"]["by_script"].items() if ms[0]["wallet_id"]=="wallet-b")
    raw["vout"].append(dict(n=1,scriptpubkey=script_b,value_sats=100))
    first["raw_json"]=json.dumps(raw); first["amount"]+=100000
    receipt=dict(first,id="receipt",wallet_id="wallet-c",direction="inbound",amount=raw["vout"][0]["value_sats"]*1000,fee=0)
    blocker=dict(first,id="blocker",wallet_id="wallet-b",direction="inbound",amount=100000,fee=0,external_id="provider",raw_json="{}")
    later=dict(first,id="later",amount=first["amount"]+1)
    book["transactions"]=[first,receipt,blocker,later]
    book["wallets"]=[w for w in book["wallets"] if w["id"]!="wallet-b"]
    for row in book["transactions"]:
        row["observation_graph_hash"]=canonical_graph_hash(row["raw_json"])
        row["observation_quantity_hash"]=canonical_observed_quantity_hash(row)
    yield "rollback_reuse",book,[]


def vectors():
    result=[]
    for case,book,handled in books():
        vector={"case":case,"input":book,"handled_ids":handled}
        try:
            derived=derive_ownership_transfers(book["transactions"],index=decode_owned_index(book["owned_index"]),wallet_refs_by_id={str(w["id"]):w for w in book["wallets"]},already_paired_ids=set(handled))
            vector["expected"]={key:sorted(value) if isinstance(value,set) else value for key,value in vars(derived).items()}
        except Exception as exc:vector["error"]=type(exc).__name__
        result.append(to_jsonable(vector,redact_config=False))
    return result




def helper_vectors():
    from kassiber.core import ownership_transfers as ref
    from kassiber.core.onchain import exact_onchain_fee_msat_from_parsed
    from tests.custody_graph_vectors import graph
    output=[]
    def add(case, op, parsed, row=None):
        row=row or {"asset":"LBTC"}
        if op=="asset": expected=ref._component_asset(parsed,row)
        elif op=="relation": expected=[ref._leg_asset_relation(leg,parsed,row) for leg in parsed["inputs"]+parsed["outputs"]]
        elif op=="inputs": expected=ref._component_inputs(parsed,row)
        else: expected=exact_onchain_fee_msat_from_parsed(parsed,asset=row.get("asset"))
        output.append(dict(case=case,op=op,graph_json=json.dumps(parsed),row=row,expected=to_jsonable(expected,redact_config=False)))
    base=graph()
    for case,component in (("empty",{}),("id",{"asset_id":" AB ","asset":" lbtc "}),("falsy",{"asset_id":False,"asset":0}),("nonfinite",{"asset_id":float("inf"),"asset":float("nan")}),("literal",{"asset_id":{"$float":"inf"},"asset":[True]})):
        add("asset_"+case,"asset",dict(base,component=component),{"asset":"BTC"})
    for case in ("id", "display", "unknown", "id_unknown", "bitcoin", "bitcoin_empty", "id_empty", "id_other_only"):
        parsed=dict(base,chain="bitcoin" if case.startswith("bitcoin") else "liquid",component={"asset_id":"ab","asset":"LBTC"} if case in {"id","id_unknown","id_empty","id_other_only"} else {"asset":"LBTC"} if case=="display" else {})
        parsed["inputs"]=[dict(asset_id="ab",asset="LBTC",value_sats=1),dict(asset_id="cd",asset="OTHER",value_sats=1),dict(asset_id=None,asset=None,value_sats=1)]
        parsed["outputs"]=[]
        if case=="id_unknown":parsed["inputs"]=[dict(asset="LBTC",value_sats=1)]
        if case=="unknown":parsed["inputs"]=[dict(value_sats=1)]
        if case in {"id_empty","bitcoin_empty"}:parsed["inputs"]=[]
        if case=="id_other_only":parsed["inputs"]=[dict(asset_id="cd",asset="OTHER",value_sats=1)]
        add("relation_"+case,"relation",parsed,{"asset":""})
        add("inputs_"+case,"inputs",parsed,{"asset":""})
    for case in ("bitcoin", "conflicts", "empty_inputs", "empty_outputs", "missing_input", "missing_output", "negative", "huge", "unknown_chain"):
        parsed=deepcopy(base)
        if case=="conflicts":parsed["evidence_conflicts"]=["x"]
        if case=="empty_inputs":parsed["inputs"]=[]
        if case=="empty_outputs":parsed["outputs"]=[]
        if case=="missing_input":parsed["inputs"][0]["value_sats"]=None
        if case=="missing_output":parsed["outputs"][0]["value_sats"]=None
        if case=="negative":parsed["inputs"][0]["value_sats"]=0
        if case=="huge":parsed["inputs"][0]["value_sats"]+=10**80
        if case=="unknown_chain":parsed["chain"]="other"
        add("fee_"+case,"fee",parsed)
    for case in ("id", "other_id", "display", "other_display", "display_fallback", "two", "none", "missing", "missing_other", "negative", "nonfinite", "literal"):
        parsed=dict(chain="liquid",component=dict(asset_id="ab",asset="LBTC"),inputs=[],outputs=[dict(role="fee",asset_id="ab",asset="LBTC",value_sats=3)])
        fee=parsed["outputs"][0]
        if case=="other_id":fee["asset_id"]="cd"
        if case in {"display","other_display","display_fallback"}:fee["asset_id"]=None
        if case=="other_display":fee["asset"]="OTHER"
        if case=="display_fallback":parsed["component"]["asset"]=None
        if case=="two":parsed["outputs"].append(dict(fee))
        if case=="none":parsed["outputs"]=[]
        if case=="missing":fee["value_sats"]=None
        if case=="missing_other":parsed["outputs"].append(dict(fee,asset_id="cd",value_sats=None))
        if case=="negative":fee["value_sats"]=-1
        if case in {"nonfinite","literal"}:
            asset=float("inf") if case=="nonfinite" else {"$float":"inf"}
            parsed["component"]["asset_id"]=asset;fee["asset_id"]=asset
        add("fee_liquid_"+case,"fee",parsed)
    for case,row in (
        ("chain",dict(raw_json='{"chain":" LIQUID "}',asset="OTHER")),
        ("component",dict(raw_json='{"component":{"asset_id":"ab"}}',asset="OTHER")),
        ("symbol",dict(raw_json="{}",asset=" l-btc ")),
        ("absent",dict(raw_json="{}",asset="BTC")),
        ("nonfinite",dict(raw_json='{"chain":NaN}',asset="BTC")),
    ):
        output.append(dict(case="row_liquid_"+case,op="row_liquid",graph_json="{}",row=row,expected=ref._row_is_liquid(row)))
    from kassiber.core.onchain import parse_ownership_tx
    for case in ("exact","mismatch","unknown_input","unknown_change","unknown_change_asset","other_change","bitcoin"):
        book=graph_snapshot(0,case="bitcoin_ownership" if case=="bitcoin" else "liquid_ownership")
        row=book["transactions"][0]
        parsed=parse_ownership_tx(row["raw_json"])
        if case in {"unknown_change","unknown_change_asset","other_change"}:
            change=dict(parsed["inputs"][0],n=2,script=parsed["inputs"][0]["script"],value_sats=5)
            if case=="unknown_change":change["value_sats"]=None
            if case=="unknown_change_asset":change["asset_id"]=None;change["asset"]=None
            if case=="other_change":change["asset_id"]="ee"*32
            parsed["outputs"].append(change)
        if case=="unknown_input":parsed["inputs"][0]["value_sats"]=None
        inputs,_=ref._component_inputs(parsed,row)
        total=row["amount"]+row["fee"]+(1 if case=="mismatch" else 0)
        scope=(parsed["chain"],parsed["network"])
        expected=ref._liquid_source_conserves(parsed,row,decode_owned_index(book["owned_index"]),str(row["wallet_id"]),inputs,total,scope)
        output.append(dict(case="conserves_"+case,op="conserves",graph_json=json.dumps(parsed),row=row,snapshot=book,total=total,scope=scope,expected=to_jsonable(expected,redact_config=False)))
    return to_jsonable(output,redact_config=False)


if __name__ == "__main__":
    for path, values in ((PATH, vectors()), (PATH.with_name("custody_ownership_helpers.json"), helper_vectors())):
        text=json.dumps(values,indent=2,ensure_ascii=True,sort_keys=True)+"\n"
        if "--write" in sys.argv: path.write_text(text)
        else: assert path.read_text()==text
