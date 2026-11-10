"""Raw books exercising successful ownership and consolidation producers.

Both producers compare production in Python and Rust through the partial seam.
"""
from copy import deepcopy
import hashlib
import json
import random

from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_interpretation_generator import recorded_snapshot
from tests.custody_graph_vectors import match
from tests.custody_liquid_vectors import compact

GRAPH_SCENARIOS = (
    "bitcoin_ownership", "liquid_ownership", "bitcoin_consolidation", "liquid_consolidation",
    "liquid_rawhex", "liquid_rawhex_short_locktime", "liquid_rawhex_flag", "liquid_rawhex_nonminimal",
)


def graph_snapshot(seed, *, case=None):
    case=case or GRAPH_SCENARIOS[seed%len(GRAPH_SCENARIOS)]
    liquid=case.startswith("liquid")
    consolidation=case.endswith("consolidation")
    chain,network,asset=("liquid","elementsregtest","LBTC") if liquid else ("bitcoin","main","BTC")
    book=recorded_snapshot(seed,case="liquid_regtest" if liquid else "bitcoin")
    rng=random.Random(20261030+seed)
    a,b=rng.randrange(10000,1000000),rng.randrange(10000,1000000)
    fee=3
    source=next(r for r in book["transactions"] if r["direction"]=="outbound")
    receipt=next(r for r in book["transactions"] if r["direction"]=="inbound")
    rows=[source]
    source.update(id="source-a",wallet_id="wallet-a",amount=a*1000,fee=fee*1000)
    if consolidation:
        rows.append(dict(source,id="source-b",wallet_id="wallet-b",amount=b*1000))
        receipt.update(id="receipt-c",wallet_id="wallet-c",amount=(a+b+fee)*1000,fee=0)
        rows.append(receipt)
    scripts={wallet:"0014"+hashlib.sha256(wallet.encode()).hexdigest()[:40] for wallet in ("wallet-a","wallet-b","wallet-c")}
    asset_id=hashlib.sha256(b"synthetic-liquid-asset").hexdigest()
    fields=dict(asset_id=asset_id,asset=asset) if liquid else {}
    inputs=[dict(txid=hashlib.sha256(f"prev:{seed}:{i}".encode()).hexdigest(),vout=i,prevout=dict(scriptpubkey=scripts["wallet-a" if i==0 else "wallet-b"],value_sats=amount+fee,**fields)) for i,amount in enumerate((a,b) if consolidation else (a,))]
    outputs=[dict(n=0,scriptpubkey=scripts["wallet-c"],value_sats=a+b+fee if consolidation else a,**fields)]
    if liquid:outputs.append(dict(n=1,scriptpubkey="",value_sats=fee,role="fee",**fields))
    raw=dict(chain=chain,network=network,txid=source["external_id"],vin=inputs,vout=outputs)
    if liquid:raw["component"]=dict(fields)
    if "rawhex" in case:
        flag=2 if case.endswith("flag") else 0
        wire=b"\x02\x00\x00\x00"+bytes([flag])+compact(len(inputs),2 if case.endswith("nonminimal") else 0)
        for entry in inputs:
            wire+=bytes.fromhex(entry["txid"])[::-1]+entry["vout"].to_bytes(4,"little")+b"\x00"+b"\xff"*4
        wire+=compact(len(outputs))
        for output in outputs:
            script=bytes.fromhex(output["scriptpubkey"])
            wire+=b"\x01"+bytes.fromhex(asset_id)[::-1]+b"\x01"+output["value_sats"].to_bytes(8,"big")+b"\x00"+compact(len(script))+script
        if not case.endswith("short_locktime"):wire+=b"\x00"*4
        raw["raw_hex"]=wire.hex()
    owned=dict(by_script={},by_outpoint={},by_scoped_outpoint=[],txid_wallets={},txid_wallets_by_scope=[])
    for wallet,script in scripts.items():
        owned["by_script"][script]=[match(wallet,chain=chain,network=network,label=wallet)]
    for entry in inputs:
        wallet="wallet-a" if entry["vout"]==0 else "wallet-b"
        point=f"{entry['txid']}:{entry['vout']}"
        owned["by_outpoint"][point]=[match(wallet,chain=chain,network=network,label=wallet)]
        owned["by_scoped_outpoint"].append(dict(key=[chain,network,point],matches=deepcopy(owned["by_outpoint"][point])))
        owned["txid_wallets"][entry["txid"]]=[[wallet,wallet]]
        owned["txid_wallets_by_scope"].append(dict(key=[chain,network,entry["txid"]],matches=[[wallet,wallet]]))
    book["owned_index"]=owned
    for wallet in book["wallets"]:
        wallet["config_json"]=json.dumps(dict(chain=chain,network=network))
    for row in rows:
        row.update(asset=asset,config_json=json.dumps(dict(chain=chain,network=network)),raw_json=json.dumps(raw),fiat_rate=None,fiat_rate_exact=None,fiat_value=None,fiat_value_exact=None)
        row.update(observation_graph_hash=canonical_graph_hash(row["raw_json"]),observation_quantity_hash=canonical_observed_quantity_hash(row))
    rows.sort(key=lambda r:(r["occurred_at"],r["created_at"],r["id"]))
    book["transactions"]=rows
    book["gap_worklist"]["rows"]=deepcopy(rows)
    return book
