from __future__ import annotations

import copy
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
sys.path.insert(0,str(Path(__file__).parent))
from generate_analysis_index import database
from kassiber.core import chain_analysis_datasets as datasets
from kassiber.errors import AppError

DEST=ROOT/"crates/wallets/tests/data/analysis_datasets.json"
AS_OF="2026-02-01T00:00:00Z"
TX="a"*64
ADDRESS="1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
SCRIPT="76a91462e907b15cbf27d5425399ebf6f0fb50ebb88f1888ac"


def insert(conn,table,row):
    conn.execute(f"INSERT INTO {table} ({','.join(row)}) VALUES ({','.join('?' for _ in row)})",tuple(row.values()))


def restored(captured):
    conn=database()
    conn.execute("DELETE FROM chain_analysis_dataset_claims")
    conn.execute("DELETE FROM chain_analysis_datasets")
    for pack in captured["packs"]:
        row={k:v for k,v in pack.items() if k not in {"manifest","accounting_authority"}}
        row.update(profile_id="p",manifest_json=json.dumps(pack["manifest"]))
        insert(conn,"chain_analysis_datasets",row)
    for row in captured["claims"]: insert(conn,"chain_analysis_dataset_claims",row)
    conn.commit()
    return conn


def evaluate(case,snapshots):
    try:
        if case["group"]=="canonical": return list(datasets.canonical_subject(*case["subject"]))
        conn=restored(snapshots[case["snapshot"]])
        try: return datasets.match_subjects(conn,"p",case["subjects"],observer=case.get("observer","owner"),limit=case.get("limit",5000),as_of=case.get("as_of",AS_OF))
        finally: conn.close()
    except AppError as exc:
        return {"error":{"code":exc.code,"message":str(exc),"details":exc.details,"retryable":exc.retryable}}


def captured():
    conn=database()
    for ident,visibility,chain,network,status in [("0-private","private","bitcoin","main","active"),("1-public","public","bitcoin","main","active"),("2-public","public","bitcoin","main","active"),("3-test","public","bitcoin","test","active"),("4-liquid","public","liquid","liquidv1","active"),("5-revoked","public","bitcoin","main","revoked")]:
        manifest=datasets.normalize_manifest({"dataset_key":ident,"name":"Research 雪","version":"v1","chain":chain,"network":network,"source":"local source","license":"LicenseRef-Example","attribution_method":"sourced_claim","visibility":visibility})
        insert(conn,"chain_analysis_datasets",{"id":ident,"profile_id":"p","dataset_key":ident,"version":"v1","chain":chain,"network":network,"visibility":visibility,"status":status,"manifest_json":json.dumps(manifest),"content_sha256":"f"*64,"claims_sha256":"e"*64,"row_count":4,"byte_count":800,"created_at":"fixed","updated_at":"fixed"})
        for record,subject in enumerate(["tx:"+TX,"tx:"+TX,"out:"+TX+":0","script:"+SCRIPT],1):
            insert(conn,"chain_analysis_dataset_claims",{"dataset_id":ident,"record_number":record,"subject":subject,"label":f"Entity 雪 {record}","category":"service","source_record_json":json.dumps({"source_id":f"row-{record}",**({"source":"specific source","attribution_method":"published_claim"} if record==2 else {})}),"valid_from":None,"valid_until":None})
    result={"packs":[datasets._pack(r) for r in conn.execute("SELECT * FROM chain_analysis_datasets ORDER BY id")],"claims":[dict(r) for r in conn.execute("SELECT * FROM chain_analysis_dataset_claims ORDER BY dataset_id,record_number")]}
    conn.close()
    return result


def vectors():
    snapshots={"base":captured()}; cases=[]
    def add(name,group="matching",snapshot="base",**kw):
        case={"name":name,"group":group,**kw}
        if group=="matching": case["snapshot"]=snapshot
        case["expected"]=evaluate(case,snapshots); cases.append(case)
    for subject in [TX,TX.upper(),"tx:"+TX,"out:"+TX,TX+":0",TX+":4294967295",TX+":4294967296",TX+":00","tx:"+TX+":0","bitcoin:main:tx:"+TX,"bitcoin:test:tx:"+TX,"script:6A00","script:","script:0","script:xz","script:"+"00"*10000,"script:"+"00"*10001,ADDRESS,"  ","wallet:private",None,12]:
        add(f"canonical-{len(cases)}",group="canonical",subject=["bitcoin","main",subject])
    for chain,network in [("bitcoin","test"),("bitcoin","mainnet"),("liquid","main"),("liquid","regtest"),("liquid","unsupported"),("lightning","main"),("bitcoin",""),("bitcoin",None)]:
        for subject in [TX,ADDRESS]: add(f"domain-{len(cases)}",group="canonical",subject=[chain,network,subject])
    from embit import ec,script
    from embit.liquid.addresses import address
    from embit.liquid.networks import NETWORKS
    sc=script.Script(bytes.fromhex("0014"+"45"*20)); blind=ec.PrivateKey(bytes.fromhex("12"*32)).get_public_key()
    for network in ["liquidv1","liquidtestnet","elementsregtest"]:
        for value in [address(sc,network=NETWORKS[network]),address(sc,blind,network=NETWORKS[network])]:
            for chosen in [network,"liquidtestnet" if network=="liquidv1" else "liquidv1"]:
                add(f"liquid-{len(cases)}",group="canonical",subject=["liquid",chosen,value])
    for observer in ["public","owner","disclosed"]:
        for limit in [1,2,3,4,5000]:
            for subjects in [[("bitcoin","main",TX)],[("bitcoin","main",ADDRESS)],[],[("bitcoin","main",TX),("bitcoin","main","tx:"+TX.upper()),("bitcoin","main",TX+":0")]]:
                add(f"match-{len(cases)}",observer=observer,limit=limit,subjects=subjects)
    for key,value in [("expired-pack","2026-01-01T00:00:00Z"),("future-pack","2027-01-01T00:00:00Z")]:
        changed=copy.deepcopy(snapshots["base"])
        changed["packs"][0]["manifest"]["valid_until" if key=="expired-pack" else "valid_from"]=value
        snapshots[key]=changed
        add(key,snapshot=key,subjects=[("bitcoin","main",TX)])
    for moment in ["2026-01-01","2026-02-01T00:00:00Z","2026-03-01T00:00:00Z"]:
        changed=copy.deepcopy(snapshots["base"])
        for row in changed["claims"]: row.update(valid_from="2026-02-01T00:00:00Z",valid_until="2026-03-01T00:00:00Z")
        snapshots["validity"]=changed
        add(f"claim-date-{moment}",snapshot="validity",as_of=moment,subjects=[("bitcoin","main",TX)])
    add("chunk-boundary",subjects=[("bitcoin","main",f"{i:064x}") for i in range(201)])
    for kw in [{"observer":"invalid"},{"limit":True},{"limit":0},{"limit":20001},{"limit":"1"}]:
        add(f"invalid-{len(cases)}",subjects=[("bitcoin","main",TX)],**kw)
    for subjects in [[["bitcoin","main"]],[["lightning","main",TX]],[["bitcoin","test",ADDRESS]]]:
        add(f"invalid-subject-{len(cases)}",subjects=subjects)
    return json.loads(json.dumps({"snapshots":snapshots,"cases":cases}))


def main():
    result=vectors()
    if "--write" in sys.argv:
        DEST.write_text("{\n\"snapshots\":{\n"+",\n".join(json.dumps(k)+":"+json.dumps(v,ensure_ascii=True,separators=(",",":")) for k,v in result["snapshots"].items())+"\n},\n\"cases\":[\n"+",\n".join(json.dumps(c,ensure_ascii=True,separators=(",",":")) for c in result["cases"])+"\n]\n}\n")
    elif json.loads(DEST.read_text())!=result: raise SystemExit("dataset vectors drifted")
    print(f"{len(result['cases'])} dataset vectors")


if __name__=="__main__": main()
