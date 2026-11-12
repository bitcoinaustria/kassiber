"""Python graph conflicts, mapping, scope and index lookup boundary witnesses."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core import onchain, ownership_transfers as transfer
from kassiber.core.custody_snapshot import decode_owned_index

PATH = Path(__file__).parent / "fixtures/custody_graph_cases.json"
TXID = "ab" * 32
OUTPOINT = "cd" * 32 + ":7"
SCRIPT = "0014" + "12" * 20
ASSET = "ef" * 32


def match(wallet="a", *, chain="bitcoin", network="main", kind="descriptor", source="inventory", label=None):
    return dict(wallet_id=wallet, wallet_label=label if label is not None else wallet.upper(), account="Assets:Bitcoin",
                chain=chain, network=network, wallet_kind=kind, source=source)


def index():
    return dict(by_script={SCRIPT: [match()]}, by_outpoint={OUTPOINT: [match()]},
                by_scoped_outpoint=[], txid_wallets={OUTPOINT[:64]: [["a", "A"]]}, txid_wallets_by_scope=[])


def graph():
    return dict(txid=TXID, chain="bitcoin", network="main", component={}, evidence_conflicts=[],
                inputs=[dict(outpoint=OUTPOINT, script=SCRIPT, value_sats=123456, asset_id="BTC", asset="BTC")],
                outputs=[dict(n=0, script="51", value_sats=123453, asset_id="BTC", asset="BTC", role=None)])


def evaluate(case):
    op = case["operation"]
    if op in ("native_delta", "native_zero", "normalize_fees"):
        book=case["input"]
        rows=deepcopy(book["transactions"])
        owned=decode_owned_index(book["owned_index"])
        if op=="native_delta":return transfer.native_wallet_delta_msat(rows[0],owned,str(rows[0]["wallet_id"]))
        if op=="native_zero":return sorted(transfer.native_zero_delta_ids(rows,owned))
        from kassiber.core.custody_evidence import _normalize_owned_bitcoin_fees, _VERIFIED_AUTHORITY_KEY
        from kassiber.core.custody_evidence import assess_authoritative_chain_observation
        for row in rows:
            authority=assess_authoritative_chain_observation(row)
            if authority.authoritative:row[_VERIFIED_AUTHORITY_KEY]=authority
        _normalize_owned_bitcoin_fees(rows)
        for row in rows:row.pop(_VERIFIED_AUTHORITY_KEY,None)
        return rows
    facts = case.get("index")
    owned = decode_owned_index(facts) if facts is not None else None
    if op == "parse": return onchain.parse_ownership_tx(case["raw"])
    if op == "merge": return onchain.merge_ownership_txs(case["graphs"])
    if op == "scope": return transfer._ownership_onchain_scope(case["row"], parsed=case["graph"], index=owned)
    if op == "parsed_scope": return transfer._parsed_chain_network_for_row(case["graph"], owned, case["row"])
    if op == "profile": return transfer._profile_parsed_transactions(case["rows"], owned)
    physical = tuple(case["scope"]) if case.get("scope") is not None else None
    if op == "outpoint":
        matches = transfer._lookup_outpoint(owned, case["value"], physical_scope=physical)
        return [{k: v for k, v in vars(m).items() if k not in ("branch_label", "address_index", "derivation_path")} for m in matches]
    if op == "txid": return sorted(transfer._lookup_txid_wallets(owned, case["value"], physical_scope=physical))
    if op == "input_owners": return sorted(transfer._input_owner_ids(owned, case["entry"], physical_scope=physical))
    if op == "source_scope": return transfer._source_chain_network(case["graph"]["inputs"], owned, case["wallet"])
    if op == "normalize": return transfer._norm_chain_network(*case["value"])
    raise AssertionError(op)


def vectors():
    cases = []
    def add(group, name, operation, **kwargs):
        case = dict(group=group, case=name, operation=operation, **deepcopy(kwargs))
        try: case["expected"] = evaluate(case)
        except Exception as exc:
            assert isinstance(exc, (AttributeError, TypeError, ValueError, OverflowError)), (name, type(exc).__name__)
            case["error"] = type(exc).__name__
        cases.append(case)
    base = graph()
    for side in ("inputs", "outputs"):
        for field in ("script", "value_sats", "asset_id", "asset", "role"):
            if side == "inputs" and field == "role": continue
            for current, incoming in ((None, "filled"), ("", "filled"), ("first", "second"), (0, 1), (False, 0), ([], {})):
                a, b = deepcopy(base), deepcopy(base)
                a[side][0][field], b[side][0][field] = current, incoming
                add("merge", f"{side}:{field}:{current!r}:{incoming!r}", "merge", graphs=[a, b, b])
    for field in ("txid", "chain", "network"):
        a = deepcopy(base); a[field] = None
        add("merge", f"fill:{field}", "merge", graphs=[a, base])
        a[field] = "conflict"
        add("merge", f"header:{field}", "merge", graphs=[base, a, a])
    for a_role, b_role in (("external", "owned"), ("owned", "external"), ("fee", "owned")):
        a,b=deepcopy(base),deepcopy(base)
        a["outputs"][0]["role"],b["outputs"][0]["role"]=a_role,b_role
        add("merge", f"role:{a_role}:{b_role}", "merge", graphs=[a,b])
    a,b=deepcopy(base),deepcopy(base)
    a["inputs"][0]["outpoint"]=None
    add("merge", "positional_rekey", "merge", graphs=[a,b])
    a["inputs"].append(deepcopy(a["inputs"][0]))
    b["inputs"].insert(0,deepcopy(b["inputs"][0]))
    b["inputs"][1]["value_sats"]+=1
    add("merge", "positional_collision", "merge", graphs=[a,b])
    a,b=deepcopy(base),deepcopy(base)
    a["evidence_conflicts"]=["original", "original"]
    b["evidence_conflicts"]=["ignored"]
    add("merge", "first_conflicts_only", "merge", graphs=[a,b])
    a=deepcopy(base)
    a["outputs"]=[dict(a["outputs"][0],n=n) for n in (10**100,7,-1,2,10)]
    add("merge","unbounded_numeric_output_order","merge",graphs=[a])
    add("merge", "empty", "merge", graphs=[])
    raw=dict(txid=TXID,vin=[dict(txid="cd"*32,vout=7,prevout=dict(scriptpubkey=SCRIPT,value_sats=123456))],
             vout=[dict(n=0,scriptpubkey="51",value_sats=123453)])
    for field in ("value_sats", "value"):
        for i,value in enumerate((None,False,True,"",0,-1,2**64,10**100,1.23456789,1e-8,"1e-8","1.000000005","1.000000015"," +١_٢٣ "," 1.0 ","NaN",[],{})):
            changed=deepcopy(raw)
            changed["vout"][0].pop("value_sats")
            changed["vout"][0][field]=value
            add("mapping", f"value:{field}:{i}", "parse", raw=changed)
    for field in ("n", "vout"):
        for i,value in enumerate((None,False,True,-1,10**100," +١_٢٣ ","bad",[],1.9)):
            changed=deepcopy(raw)
            changed["vout" if field=="n" else "vin"][0][field]=value
            add("mapping",f"index:{field}:{i}","parse",raw=changed)
    for field in ("n", "vout", "value_sats"):
        for digits in (4300, 4301):
            changed=deepcopy(raw)
            changed["vin" if field=="vout" else "vout"][0][field]="1"*digits
            add("mapping",f"integer_digit_limit:{field}:{digits}","parse",raw=changed)
    # Keep nonfinite scalars inside stored JSON text: outer to_jsonable must
    # not replace the very types whose admission and exception order we pin.
    for location in ("direct", "tx", "ownership_graph"):
        for field in ("n", "vout", "value_sats", "value", "scriptpubkey", "asset", "asset_id", "role", "txid", "chain", "network", "component"):
            for spelling, value in (("nan", float("nan")), ("inf", float("inf")), ("negative_inf", -float("inf"))):
                changed=deepcopy(raw)
                if field=="vout":changed["vin"][0][field]=value
                elif field in ("txid", "chain", "network", "component"):changed[field]=value
                else:
                    if field=="value":changed["vout"][0].pop("value_sats")
                    changed["vout"][0][field]=value
                if location!="direct":changed={location:changed}
                add("mapping_nonfinite",f"{location}:{field}:{spelling}","parse",raw=json.dumps(changed))
    for scenario in ("invalid_txid", "coinbase_txid", "missing_txid", "input_before_output", "output_before_script", "direct_before_nested", "nested_before_owned", "no_shape", "nan_fallback"):
        changed=deepcopy(raw)
        changed["vin"][0]["vout"]=float("inf")
        changed["vout"][0].update(n=float("inf"), scriptpubkey=[float("nan")])
        if scenario=="invalid_txid":changed["vin"][0]["txid"]="invalid";changed["vout"]=[]
        if scenario=="coinbase_txid":changed["vin"][0]["txid"]="0"*64;changed["vout"]=[]
        if scenario=="missing_txid":changed["vin"][0].pop("txid");changed["vout"]=[]
        if scenario=="output_before_script":changed["vin"]=[]
        if scenario=="direct_before_nested":changed["tx"]={"vin":[],"vout":[{"n":float("nan")} ]}
        if scenario=="nested_before_owned":changed={"tx":changed,"ownership_graph":raw}
        if scenario=="no_shape":changed.pop("vout")
        if scenario=="nan_fallback":changed["vin"][0]["vout"]=float("nan");changed["vout"][0]["n"]=float("nan")
        add("mapping_admission",scenario,"parse",raw=json.dumps(changed))
    for scenario in ("scalar_nan", "list_nan", "dict_nan", "nan_vs_literal_tag", "infinite_vs_large_int", "float_rounding", "asset_whitespace"):
        changed=deepcopy(raw)
        if scenario=="asset_whitespace":changed["vout"][0]["asset"]="ab "*20+"abcd"
        else:
            a={"scalar_nan":float("nan"),"list_nan":[float("nan")],"dict_nan":{"a":float("nan")},"nan_vs_literal_tag":float("nan"),"infinite_vs_large_int":float("inf"),"float_rounding":float(2**100)}[scenario]
            b={"nan_vs_literal_tag":{"$float":"nan"},"infinite_vs_large_int":10**400,"float_rounding":2**100+1}.get(scenario,deepcopy(a))
            changed["vout"][0]["scriptpubkey"]=a
            changed["ownership_graph"]=deepcopy(raw)
            changed["ownership_graph"]["vout"][0]["scriptpubkey"]=b
        add("mapping_admission",scenario,"parse",raw=json.dumps(changed))
    for i, whitespace in enumerate(("\t", "\n", "\v", "\f", "\r", "\x1c", "\u0085")):
        changed=deepcopy(raw)
        changed["vout"][0]["asset"]="ab"+whitespace+"ab "*19+"abcd"
        add("mapping_admission",f"asset_hex_whitespace:{i}","parse",raw=json.dumps(changed))
    changed=deepcopy(raw)
    changed["vout"][0]["asset"]="a b"*20+"abcd"
    add("mapping_admission","asset_hex_split_byte","parse",raw=json.dumps(changed))
    for i,script in enumerate(("script_hex", "nested_hex", "nested_fallback", "nested_string", "empty", "missing")):
        changed=deepcopy(raw);out=changed["vout"][0];out.pop("scriptpubkey")
        if script=="script_hex":out["script_hex"]="51"
        if script=="nested_hex":out["scriptPubKey"]={"hex":"51"}
        if script=="nested_fallback":out["scriptPubKey"]={"hex":"","scriptpubkey":"51"}
        if script=="nested_string":out["scriptPubKey"]="51"
        if script=="empty":out["scriptpubkey"]=""
        add("mapping",f"script:{i}","parse",raw=changed)
    for value in (0,False,"AB"*32,"btc","ſ",{"a":1}):
        changed=deepcopy(raw);changed["vout"][0]["asset"]=value
        add("mapping",f"asset:{value!r}","parse",raw=changed)
    from tests.custody_liquid_vectors import parts, encoded
    for chain in ("liquid", "bitcoin", ""):
        changed=deepcopy(raw);changed["chain"]=chain
        changed["component"]={"asset_id":ASSET} if chain=="" else {}
        changed["raw_hex"]=encoded(parts()).hex()
        add("mapping",f"raw_hex:{chain}","parse",raw=changed)
    # Explicit scope recovery is forced past the ordinary scope predicate by
    # the synthetic id. The same helper serves these retained graph anchors.
    row=dict(id="owned-derive:source",wallet_id="a",asset="BTC",external_id=TXID,config_json="{}")
    for chain_name,network_name,asset in (("bitcoin","main","BTC"),("liquid","elementsregtest","LBTC")):
        for scenario in ("inputs", "outputs", "ambiguous_inputs", "ambiguous_outputs", "contradict_chain", "contradict_network", "invalid_chain", "invalid_network", "no_index_scope", "explicit", "explicit_chain_only", "config_fallback", "display_mismatch", "missing_asset_id", "bad_asset_id", "txid_mismatch", "bad_txid", "external_only", "missing_txid", "no_index", "provider_id", "nonstring_script"):
            g=deepcopy(base);g.update(chain=chain_name,network="",component={"asset_id":ASSET,"asset":asset} if chain_name=="liquid" else {})
            r={**row,"asset":asset}
            owned=index();owned["by_outpoint"]={OUTPOINT:[match(chain=chain_name,network=network_name)]}
            owned["by_script"][SCRIPT]=[match(chain=chain_name,network=network_name)]
            owned["by_script"]["51"]=[match(chain=chain_name,network=network_name)]
            if scenario in ("outputs","ambiguous_inputs","ambiguous_outputs","no_index_scope"):
                owned["by_outpoint"]={};owned["by_script"][SCRIPT]=[]
            if scenario in ("ambiguous_inputs","ambiguous_outputs"):
                key=SCRIPT if scenario=="ambiguous_inputs" else "51"
                owned["by_script"][key]=[match(chain=chain_name,network=network_name),match(chain="bitcoin",network="signet")]
            if scenario=="no_index_scope":owned["by_script"]={}
            if scenario=="contradict_chain":r["config_json"]=json.dumps({"chain":"liquid" if chain_name=="bitcoin" else "bitcoin"})
            if scenario=="contradict_network":g["network"]="test";r["config_json"]='{"network":"regtest"}'
            if scenario=="invalid_chain":g["chain"]="lightning"
            if scenario=="invalid_network":g["network"]="unknown"
            if scenario in ("explicit","explicit_chain_only"):
                owned["by_script"]={};owned["by_outpoint"]={}
                if scenario=="explicit":g["network"]=network_name
            if scenario=="config_fallback":r.update(config_json="",wallet_config_json=json.dumps({"chain":chain_name,"network":network_name}))
            if scenario=="display_mismatch":g["component"]={"asset_id":ASSET,"asset":"BTC" if chain_name=="liquid" else "LBTC"}
            if scenario in ("missing_asset_id","bad_asset_id"):
                g["component"].pop("asset_id",None)
                if scenario=="bad_asset_id":g["component"]["asset_id"]="bad"
            if scenario=="txid_mismatch":r["external_id"]="12"*32
            if scenario=="bad_txid":g["txid"]="provider-id"
            if scenario=="external_only":g["txid"]=None
            if scenario=="missing_txid":g["txid"]=None;r["external_id"]=None
            if scenario=="provider_id":r["external_id"]="provider-id"
            if scenario=="no_index":owned=None
            if scenario=="nonstring_script":g["outputs"][0]["script"]=123;owned["by_outpoint"]={};owned["by_script"][SCRIPT]=[]
            add("scope",f"{chain_name}:{scenario}","scope",row=r,graph=g,index=owned)
    for c in (None,False,"", " BTC ","elements","ln","bad"):
        for n in (None,False,"", " MAINNET ","regtest","unknown"):
            add("scope",f"normalize:{c}:{n}","normalize",value=[c,n])
    for scenario in ("no_chain", "no_metadata", "inferred_chain", "output_inference", "input_precedes_output", "source_conflict", "source_other_wallet", "config_only", "inferred_chain_contradiction", "inferred_network_contradiction", "invalid_id_precedes_script", "missing_id_reads_script"):
        g=deepcopy(base);owned=index();r=deepcopy(row)
        if scenario in ("no_chain","no_metadata","inferred_chain","output_inference"):
            g["chain"]="";g["network"]="";r["asset"]="LBTC"
            owned["by_outpoint"]={};owned["by_script"]={}
            if scenario=="no_chain":g["network"]="regtest"
            if scenario in ("inferred_chain","output_inference"):
                owned["by_script"][SCRIPT if scenario=="inferred_chain" else "51"]=[match(chain="liquid",network="elementsregtest")]
        if scenario=="source_conflict":
            owned["by_outpoint"][OUTPOINT].append(match(network="regtest"))
            owned["by_script"]["51"]=[match(network="test")]
        if scenario=="source_other_wallet":owned["by_outpoint"][OUTPOINT]=[match("other")]
        if scenario=="input_precedes_output":owned["by_script"]["51"]=[match(network="test")]
        if scenario=="config_only":
            owned["by_outpoint"]={};owned["by_script"]={};g["chain"]="liquid";g["network"]=""
            r.update(config_json="",wallet_config_json='{"chain":"liquid","network":"elementsregtest"}')
        if scenario=="inferred_chain_contradiction":g["chain"]="";g["network"]="";r["config_json"]='{"chain":"liquid"}'
        if scenario=="inferred_network_contradiction":g["network"]="test"
        if scenario in ("invalid_id_precedes_script","missing_id_reads_script"):
            g["txid"]="bad" if scenario=="invalid_id_precedes_script" else None
            r["external_id"]=None;owned["by_outpoint"]={};g["inputs"][0]["script"]=123
        add("scope",scenario,"scope" if scenario.endswith("script") else "parsed_scope",graph=g,row=r,index=owned)
    # Missing source metadata must remain distinguishable from an unrelated
    # wallet's matching scope, and ambiguous outputs must not default Bitcoin.
    for scenario in ("source_other_wallet_unstamped", "ambiguous_outputs_unstamped"):
        g=deepcopy(base);g.update(chain="",network="")
        owned=index()
        owned["by_outpoint"][OUTPOINT]=[match("other")]
        owned["by_script"]["51"]=[] if scenario.startswith("source") else [match(),match(network="regtest")]
        add("scope",scenario,"parsed_scope",graph=g,row=row,index=owned)
    for scenario in ("fill","scope_separation","component_first","conflict","duplicate_id"):
        rows=[]
        for i in range(2):
            payload=deepcopy(raw)
            payload.update(chain="liquid",network="elementsregtest",component=dict(asset_id=ASSET,asset="LBTC"))
            payload["vin"][0]["prevout"].update(asset_id=ASSET,asset="LBTC")
            payload["vout"][0].update(asset_id=ASSET,asset="LBTC",role="external" if i==0 else "owned")
            if scenario=="fill" and i==0:payload["vout"][0].pop("value_sats")
            if scenario=="scope_separation" and i==1:payload["network"]="liquidv1"
            if scenario=="component_first" and i==1:payload["component"]["asset"]="OTHER"
            if scenario in ("conflict","duplicate_id") and i==1:payload["vout"][0]["value_sats"]+=1
            rows.append({**row,"id":"owned-derive:"+str(i if scenario!="duplicate_id" else 0),"asset":"LBTC","raw_json":json.dumps(payload)})
        # The explicit Liquid scopes must not be contradicted by the default
        # Bitcoin ownership fixture. These rows exercise sibling graph merges.
        owned=index();owned["by_outpoint"]={};owned["by_script"]={}
        add("profile",scenario,"profile",rows=rows,index=owned)
        assert len(cases[-1]["expected"]) == (1 if scenario=="duplicate_id" else 2), scenario
    for scenario in ("scalar_nan", "list_nan", "dict_nan", "literal_tag"):
        rows=[]
        for i in range(2):
            payload=deepcopy(raw)
            value={"scalar_nan":float("nan"), "list_nan":[float("nan")], "dict_nan":{"a":float("nan")}, "literal_tag":float("nan") if i==0 else {"$float":"nan"}}[scenario]
            payload["vout"][0]["scriptpubkey"]=value
            rows.append({**row,"id":"owned-derive:"+str(i),"raw_json":json.dumps(payload)})
        add("profile","nonfinite:"+scenario,"profile",rows=rows,index=index())
    from tests.custody_snapshot_generator import snapshot, SCENARIOS
    from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
    for scenario in ("huge_zero", "negative_huge_zero", "huge_delta", "float_fee", "false_fee", "nan_fee", "infinite_fee", "string_fee", "fee_before_script", "bad_script", "nan_script", "missing_value_before_script", "negative_fee_before_script", "missing_outpoint", "no_index_bad_amount", "amount_before_fee", "direction_before_amount", "exact_large_float_fee", "rounded_large_float_fee", "no_index", "untrusted", "missing_graph", "scope_mismatch", "empty_inputs", "empty_outputs", "conflict", "ambiguous_owner", "other_owner", "input_missing_script", "output_missing_script"):
        book=snapshot(SCENARIOS.index("zero_delta"))
        row=book["transactions"][0];payload=json.loads(row["raw_json"])
        if scenario in ("huge_zero","negative_huge_zero"):
            payload["vin"][0]["prevout"]["value_sats"]=payload["vout"][0]["value_sats"]=(1 if scenario=="huge_zero" else -1)*10**100
        if scenario in ("huge_delta","exact_large_float_fee","rounded_large_float_fee"):
            extra=2**200+(scenario=="rounded_large_float_fee")
            payload["vin"][0]["prevout"]["value_sats"]+=extra
            if scenario!="huge_delta":payload["fee"]=float(2**200)
        if scenario in ("float_fee","false_fee","nan_fee","infinite_fee","string_fee"):
            payload["fee"]={"float_fee":0.0,"false_fee":False,"nan_fee":float("nan"),"infinite_fee":float("inf"),"string_fee":"0"}[scenario]
        if scenario in ("fee_before_script","bad_script","nan_script","missing_value_before_script","negative_fee_before_script"):
            payload["vout"][0]["scriptpubkey"]=float("nan") if scenario=="nan_script" else 123
            if scenario=="fee_before_script":payload["fee"]=1
            if scenario=="missing_value_before_script":payload["vin"][0]["prevout"].pop("value_sats")
            if scenario=="negative_fee_before_script":payload["vout"][0]["value_sats"]+=1
        if scenario=="missing_outpoint":payload["vin"][0]["txid"]="bad"
        if scenario=="no_index":book["owned_index"]=None
        if scenario=="untrusted":row["observation_authority_version"]=None
        if scenario=="missing_graph":payload={"txid":payload["txid"]}
        if scenario=="scope_mismatch":row["config_json"]='{"chain":"liquid"}'
        if scenario=="empty_inputs":payload["vin"]=[];payload["vout"][0]["value_sats"]=0
        if scenario=="empty_outputs":payload["vout"]=[];payload["vin"][0]["prevout"]["value_sats"]=0
        if scenario=="conflict":
            payload["ownership_graph"]=deepcopy(payload)
            payload["ownership_graph"]["vout"][0]["value_sats"]+=1
        if scenario in ("ambiguous_owner","other_owner"):
            for matches in book["owned_index"]["by_script"].values():
                if scenario=="ambiguous_owner":matches.append({**matches[0],"wallet_id":"other"})
                else:matches[0]["wallet_id"]="other"
            if scenario=="other_owner":payload["vin"][0]["prevout"]["value_sats"]+=1
        if scenario=="input_missing_script":payload["vin"][0]["prevout"].pop("scriptpubkey")
        if scenario=="output_missing_script":payload["vout"][0].pop("scriptpubkey")
        if scenario=="no_index_bad_amount":book["owned_index"]=None;row["amount"]="bad"
        if scenario=="amount_before_fee":row.update(amount=1,fee=["bad"])
        if scenario=="direction_before_amount":row.update(direction="inbound",amount="bad")
        row["raw_json"]=json.dumps(payload)
        # These short-circuit witnesses must refuse/skip before consulting the
        # observation commitment; invalid row amounts cannot be re-attested.
        if scenario not in ("no_index_bad_amount","amount_before_fee","direction_before_amount"):
            row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]),observation_quantity_hash=canonical_observed_quantity_hash(row))
        add("arithmetic_native",scenario,"native_delta" if scenario in ("huge_delta","exact_large_float_fee","rounded_large_float_fee","other_owner","input_missing_script","output_missing_script") else "native_zero",input=book)
        if scenario in ("huge_zero","negative_huge_zero","float_fee","false_fee"):
            assert cases[-1]["expected"]==["zero"], scenario
        if scenario in ("huge_delta","exact_large_float_fee"):
            assert cases[-1]["expected"]==2**200*1000, scenario
    for scenario in ("huge_cancel", "shape", "float_fee", "false_fee", "nan_fee", "input_list", "output_dict", "numeric_script", "empty_script", "missing_graph_before_overflow", "missing_value_before_unhashable", "missing_outpoint_before_unhashable", "fee_fifo", "fee_zero", "untrusted", "liquid", "bad_attribution", "nonzero_fee", "included_fee", "duplicate_wallet", "missing_graph", "conflict", "missing_input_value", "missing_output_value", "ambiguous_script", "negative_network_fee", "row_delta", "recorded_fee", "insufficient_out", "reverse_order", "group_order", "ambiguous_output", "inbound_fee", "no_outbound"):
        book=snapshot(SCENARIOS.index("shared_fee"))
        for i,row in enumerate(book["transactions"]):
            payload=json.loads(row["raw_json"])
            if scenario=="huge_cancel":
                for vin in payload["vin"]:
                    vin["prevout"]["value_sats"]+=10**100
                    payload["vout"].append(dict(n=len(payload["vout"]),scriptpubkey=vin["prevout"]["scriptpubkey"],value_sats=10**100))
            if scenario=="shape":payload["vout"][0].update(n=10**100+i,value_sats=0);payload.pop("fee")
            if scenario=="float_fee":payload["fee"]=float(payload["fee"])
            if scenario in ("false_fee","fee_zero"):
                payload["vout"][0]["value_sats"]=sum(v["prevout"]["value_sats"] for v in payload["vin"])
                payload["fee"]=False if scenario=="false_fee" else 0
            if scenario=="nan_fee":payload["fee"]=float("nan")
            if scenario=="untrusted":row["observation_authority_version"]=None
            if scenario=="no_outbound":row["direction"]="inbound";payload["vout"][0]["n"]=float("inf")
            if scenario=="liquid":payload.update(chain="liquid",network="elementsregtest");row["asset"]="LBTC"
            if scenario=="bad_attribution":row["observation_fee_attribution"]="exact"
            if scenario=="nonzero_fee":
                row["fee"]=1
                payload["vin"][0]["prevout"]["scriptpubkey"]=["unhashable"]
            if scenario=="included_fee":row["amount_includes_fee"]=1
            if scenario=="duplicate_wallet":row["wallet_id"]=book["transactions"][0]["wallet_id"];row["amount"]*=2
            if scenario=="missing_graph":payload.pop("vout")
            if scenario=="conflict" and i==1:payload["vout"][0]["value_sats"]+=1
            if scenario=="missing_input_value":payload["vin"][0]["prevout"].pop("value_sats")
            if scenario=="missing_output_value":payload["vout"][0].pop("value_sats")
            if scenario=="ambiguous_script":payload["observer_owned_scripts"].append(payload["vin"][1-i]["prevout"]["scriptpubkey"])
            if scenario=="negative_network_fee":payload["vout"][0]["value_sats"]+=payload["fee"]+1;payload.pop("fee")
            if scenario=="row_delta":row["amount"]+=1000
            if scenario=="recorded_fee":payload["fee"]+=1
            if scenario in ("ambiguous_output","inbound_fee"):
                if scenario=="ambiguous_output":payload["observer_owned_scripts"].append(payload["vout"][0]["scriptpubkey"])
                else:payload["vout"][0]["scriptpubkey"]=payload["vin"][0]["prevout"]["scriptpubkey"]
                if i==0:
                    row.update(direction="inbound",amount=row["amount"]-payload["fee"]*1000,fee=1 if scenario=="inbound_fee" else 0)
            if scenario in ("input_list","numeric_script","empty_script","missing_value_before_unhashable","missing_outpoint_before_unhashable"):
                payload["vin"][0]["prevout"]["scriptpubkey"]={"input_list":["bad"],"numeric_script":float("nan"),"empty_script":[],"missing_value_before_unhashable":["bad"],"missing_outpoint_before_unhashable":["bad"]}[scenario]
                if scenario=="missing_value_before_unhashable":payload["vin"][0]["prevout"].pop("value_sats")
                if scenario=="missing_outpoint_before_unhashable":payload["vin"][0]["txid"]="bad"
            if scenario=="output_dict":payload["vout"][0]["scriptpubkey"]={"bad":1}
            if scenario=="missing_graph_before_overflow":
                if i==0:payload.pop("vout")
                else:payload["vout"][0]["n"]=float("inf")
            if scenario=="fee_fifo":payload["vout"][0]["value_sats"]=0;payload["fee"]=sum(v["prevout"]["value_sats"] for v in payload["vin"])
            if scenario=="insufficient_out":payload["vout"][0]["value_sats"]=-1;payload["fee"]=sum(v["prevout"]["value_sats"] for v in payload["vin"])+1
            row["raw_json"]=json.dumps(payload)
            row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]),observation_quantity_hash=canonical_observed_quantity_hash(row))
        if scenario=="reverse_order":book["transactions"].reverse()
        if scenario=="group_order":
            book["transactions"]+=deepcopy(book["transactions"])
            for i,row in enumerate(book["transactions"]):
                payload=json.loads(row["raw_json"])
                row["id"]+=str(i)
                row["external_id"]=payload["txid"]=("ff" if i<2 else "00")*32
                if i<2:payload["vin"][0]["prevout"]["scriptpubkey"]=["unhashable"]
                else:payload["vout"][0]["n"]=float("inf")
                row["raw_json"]=json.dumps(payload)
                row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]),observation_quantity_hash=canonical_observed_quantity_hash(row))
        add("arithmetic_fees",scenario,"normalize_fees",input=book)
        if "expected" in cases[-1]:
            normalized=all(row.get("custody_native_fee_normalized") for row in cases[-1]["expected"])
            assert normalized==(scenario in ("huge_cancel","float_fee","false_fee","fee_fifo","fee_zero","reverse_order")), scenario
    # Index lookup ordering, empty scoped entries and same-spelling network collisions.
    for scenario in ("unscoped","legacy","scoped","scoped_empty","script_fallback","outpoint_wins","other_network","duplicates","no_scope_txid_fallback","empty_key","nonstring_script","sort_kind","sort_source","sort_label","sort_id"):
        owned=index();entry=dict(outpoint=OUTPOINT,script=SCRIPT);physical=["bitcoin","main"]
        if scenario=="unscoped":physical=None
        if scenario in ("scoped","scoped_empty","duplicates"):
            owned["by_scoped_outpoint"]=[dict(key=["bitcoin","main",OUTPOINT],matches=[] if scenario=="scoped_empty" else [match("b")])]
        if scenario=="duplicates":owned["by_scoped_outpoint"].insert(0,dict(key=["bitcoin","main",OUTPOINT],matches=[match("c")]))
        if scenario in ("script_fallback","nonstring_script"):owned["by_outpoint"]={}
        if scenario=="outpoint_wins":owned["by_script"][SCRIPT]=[match("b")]
        if scenario=="other_network":owned["by_outpoint"][OUTPOINT]=[match(network="regtest")]
        if scenario=="empty_key":entry["outpoint"]=""
        if scenario=="nonstring_script":entry["script"]=123
        if scenario.startswith("sort_"):
            owned["by_outpoint"][OUTPOINT]=[match("z"),match("b"),match("a")]
            if scenario=="sort_kind":owned["by_outpoint"][OUTPOINT][0].update(wallet_kind="address",source="derived",wallet_label="0")
            if scenario=="sort_source":owned["by_outpoint"][OUTPOINT][0].update(source="derived")
            if scenario=="sort_label":owned["by_outpoint"][OUTPOINT][0].update(wallet_label="0")
            if scenario=="sort_id":
                for m in owned["by_outpoint"][OUTPOINT]:m["wallet_label"]="same"
        add("index",scenario+":outpoint","outpoint",index=owned,value=entry["outpoint"],scope=physical)
        add("index",scenario+":txid","txid",index=owned,value=OUTPOINT[:64],scope=physical)
        if physical:add("index",scenario+":owners","input_owners",index=owned,entry=entry,scope=physical)
    for scenario in ("positive","empty","duplicates","alias","blank_chain_invalid_network"):
        owned=index();scope=["bitcoin","main"]
        if scenario=="alias":scope=[" BTC "," MAINNET "]
        if scenario=="blank_chain_invalid_network":scope=["","unknown"]
        key=["bitcoin","unknown" if scenario=="blank_chain_invalid_network" else "main",OUTPOINT[:64]]
        owned["txid_wallets_by_scope"]=[dict(key=key,matches=[] if scenario=="empty" else [["b","B"],["b","B"],["a","A"]])]
        if scenario=="duplicates":owned["txid_wallets_by_scope"].insert(0,dict(key=key,matches=[["first","First"]]))
        add("index","txid:"+scenario,"txid",index=owned,value=OUTPOINT[:64].upper(),scope=scope)
    return cases


if __name__ == "__main__":
    from tests.custody_snapshot import to_jsonable
    cases=to_jsonable(vectors(),redact_config=False)
    if "--write" in sys.argv:PATH.write_text(json.dumps(cases,indent=2,ensure_ascii=True)+"\n")
    else:assert cases==json.loads(PATH.read_text())
    print(len(cases))
