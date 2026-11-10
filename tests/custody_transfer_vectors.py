"""CPython transfer identity and matching vectors, including refused metadata."""
from copy import deepcopy
import json
from pathlib import Path
import sys
from kassiber.transfers import onchain_transfer_scope, bitcoin_network_domain, is_lightning_payment_hash_row, detect_intra_transfers
PATH = Path(__file__).parent / "fixtures/custody_transfer_cases.json"


def vectors():
    out = dict(id="source", wallet_id="a", wallet_kind="descriptor", asset="BTC", amount=123_456, direction="outbound", external_id="a" * 64, external_id_kind="txid", config_json='{"chain":"bitcoin","network":"main"}', raw_json='{"txid":"' + 'a' * 64 + '"}')
    incoming = {**out, "id":"target", "wallet_id":"b", "direction":"inbound"}
    variants = [
        ("ordinary", {}),
        ("amount_null", {"amount":None}),
        ("amount_false", {"amount":False}),
        ("amount_empty", {"amount":""}),
        ("amount_empty_list", {"amount":[]}),
        ("amount_empty_dict", {"amount":{}}),
        ("amount_huge", {"amount":10**100}),
        ("amount_string", {"amount":"123"}),
        ("amount_bad_list", {"amount":[1]}),
        ("graph_nonfinite_id", {"raw_json":'{"txid":1e400}'}),
        ("graph_nan_id", {"raw_json":'{"txid":NaN}'}),
        ("graph_infinite_chain", {"raw_json":'{"txid":"'+'a'*64+'","chain":Infinity}'}),
        ("graph_infinite_network", {"raw_json":'{"txid":"'+'a'*64+'","network":-Infinity}'}),
        ("graph_conflict", {"raw_json":json.dumps({"txid":"b"*64})}),
        ("graph_invalid_id", {"raw_json":'{"txid":"provider"}'}),
        ("graph_nested_conflict", {"raw_json":json.dumps({"txid":"a"*64,"tx":{"txid":"b"*64}})}),
        ("graph_list", {"raw_json":json.dumps([{"txid":"a"*64},{"txid":"a"*64}])}),
        ("graph_list_conflict", {"raw_json":json.dumps([{"txid":"a"*64},{"txid":"b"*64}])}),
        ("typed_unicode_key", {"raw_json":json.dumps({"Ｔx_Id":"a"*64}),"external_id_kind":None}),
        ("typed_key", {"raw_json":json.dumps({"Transaction_Hash":"a"*64}),"external_id_kind":None}),
        ("untyped_id", {"raw_json":"{}","external_id_kind":None}),
        ("provider_id", {"external_id":"provider","raw_json":"{}"}),
        ("unsupported_chain", {"config_json":'{"chain":"ln","network":"main"}'}),
        ("unsupported_network", {"config_json":'{"chain":"bitcoin","network":"unknown"}'}),
        ("whitespace_network", {"config_json":'{"network":" ","bitcoin_network":"main"}'}),
        ("network_contradiction", {"raw_json":json.dumps({"txid":"a"*64,"network":"regtest"})}),
        ("network_fallback", {"config_json":None,"wallet_config_json":'{"network":"signet"}'}),
        ("implicit_main", {"config_json":"{}"}),
        ("boolean_chain", {"config_json":'{"chain":false,"network":"main"}'}),
        ("synthetic", {"id":"recorded-fanout:source"}),
        ("liquid_no_network", {"asset":"LBTC","config_json":'{"chain":"liquid"}',"raw_json":json.dumps({"txid":"a"*64,"component":{"asset_id":"b"*64,"asset":"LBTC"}})}),
        ("liquid_identity", {"asset":"LBTC","config_json":'{"chain":"liquid","network":"liquidv1"}',"raw_json":json.dumps({"txid":"a"*64,"component":{"asset_id":"b"*64,"asset":"L-BTC"}})}),
        ("liquid_display_conflict", {"asset":"LBTC","config_json":'{"chain":"liquid","network":"liquidv1"}',"raw_json":json.dumps({"txid":"a"*64,"component":{"asset_id":"b"*64,"asset":"OTHER"}})}),
        ("liquid_identity_conflict", {"asset":"LBTC","config_json":'{"chain":"liquid","network":"liquidv1"}',"raw_json":json.dumps({"txid":"a"*64,"asset_id":"c"*64,"component":{"asset_id":"b"*64}})}),
    ]
    for kind in ("lnd", "cln"):
        cln = kind == "cln"
        native = dict(wallet_kind=kind, payment_hash="b"*64, payment_hash_source="core_lightning" if cln else "lnd", kind="cln_pay" if cln else "lnd_pay", config_json='{"network":"main"}', raw_json=json.dumps({"_kassiber_provenance":{"import_source":"core-lightning" if cln else "lnd"}}))
        variants.extend([(kind+"_native",native),(kind+"_forged",{**native,"raw_json":"{}"}), (kind+"_wrong_kind",{**native,"kind":"chain_send"}), (kind+"_chain_source",{**native,"payment_hash_source":"chain_script"}), (kind+"_no_network",{**native,"config_json":"{}"})])
    result=[]
    for case, changes in variants:
        rows=[{**deepcopy(out),**changes},deepcopy(incoming)]
        try: pairs, matched=detect_intra_transfers(rows)
        except Exception as exc:
            result.append(dict(case=case, rows=rows, error_code=type(exc).__name__))
            continue
        result.append(dict(case=case,rows=rows,scopes=[onchain_transfer_scope(r) for r in rows],domains=[bitcoin_network_domain(r) for r in rows],lightning=[is_lightning_payment_hash_row(r) for r in rows],pairs=pairs,matched_ids=sorted(matched)))
    return json.loads(json.dumps(result))

if __name__ == "__main__":
    text=json.dumps(vectors(),indent=2,sort_keys=True)+"\n"
    if "--write" in sys.argv: PATH.write_text(text)
    else: assert PATH.read_text()==text
