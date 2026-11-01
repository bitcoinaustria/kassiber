"""Freeze unchanged Python's public-feature boundary, without wallet or network I/O."""
import argparse
from copy import deepcopy
from hashlib import new, sha256
import json
from pathlib import Path
import random
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.chain_analysis.features import (
    extract_transaction_features, evaluate_features, normalize_persisted_features,
    script_type, signature_encoding,
)

DEST = ROOT / "crates/wallets/tests/data/analysis_features.json"
ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141


def evaluate(case):
    op = case["operation"]
    if op == "extract":
        return extract_transaction_features(case["raw"], source=case.get("source", "stored_transaction"),
            subject_id=case.get("subject"), signature_observations=case.get("signatures", ()))
    if op == "normalize":
        return normalize_persisted_features(case["snapshot"], source=case.get("source", "stored_transaction"), subject_id=case.get("subject"))
    if op == "evaluate":
        return evaluate_features(case["snapshot"], collaboration=case.get("collaboration"))
    if op == "script":
        return script_type(case["value"])
    if op == "signature":
        return signature_encoding(case["value"], schnorr=case.get("schnorr", False))
    raise AssertionError(op)


def der(r=1, s=1, flag=1):
    def encode(n):
        data = n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big")
        if data[0] & 128:
            data = b"\0" + data
        return b"\2" + bytes([len(data)]) + data
    payload = encode(r) + encode(s)
    return b"\x30" + bytes([len(payload)]) + payload + bytes([flag])


def push(data, width=None):
    if width is None:
        return bytes([len(data)]) + data
    return bytes([{1: 76, 2: 77, 4: 78}[width]]) + len(data).to_bytes(width, "little") + data


def h160(data):
    return new("ripemd160", sha256(data).digest()).digest()


def vectors():
    cases = []
    def add(group, name, operation, **kw):
        case = dict(group=group, name=name, operation=operation, **kw)
        case["expected"] = evaluate(case)
        cases.append(case)
    def raw(name, **kw):
        add("extract", name, "extract", **kw)
    scripts = ["76a914"+"11"*20+"88ac", "a914"+"11"*20+"87", "0014"+"11"*20,
               "0020"+"11"*32, "5120"+"11"*32, "6a0161", "21"+"02"+"11"*32+"ac",
               "41"+"04"+"11"*64+"ac", "5202abcd", "0002abcd", "", "51", None, True,
               "00 14 "+"11"*20, "0 0", "0\u00a00", "xx", "00\t14"+"11"*20]
    for i, value in enumerate(scripts):
        add("script", f"script-{i}", "script", value=value)
    for length in (3,4,21,22,23,24,25,26,33,34,35,41,42,43,66,67,68):
        for prefix in ("00", "51", "60", "61", "6a"):
            add("script", f"script-length-{length}-{prefix}", "script", value=prefix+f"{length-2:02x}"+"00"*(length-2))
    for opcode in range(256):
        if opcode in (0,0x51,0x60,0x6a):
            continue
        add("script",f"script-opcode-{opcode}","script",value=f"{opcode:02x}02abcd")
    add("script","script-length-cap","script",value="00"*100001)
    sig = der()
    for r in (0,1,2**255-1,2**255,ORDER-1,ORDER,ORDER+1):
        for s in (0,1,ORDER//2,ORDER//2+1,ORDER-1,ORDER):
            for flag in (1,3,0x81):
                add("signature",f"der-{r}-{s}-{flag}","signature",value=der(r,s,flag).hex())
    for i in range(len(sig)):
        for replacement in (0,0xff):
            value = bytearray(sig)
            value[i] = replacement
            add("signature", f"der-byte-{i}-{replacement}", "signature", value=value.hex())
    for length in (0,8,9,63,64,65,66,73,74):
        for flag in (0,1,2,3,4,0x80,0x81,0x82,0x83,0xff):
            add("signature", f"schnorr-{length}-{flag}", "signature", value=(b"\x11"*max(0,length-1)+bytes([flag]) if length else b"").hex(), schnorr=True)
    for empty in ({}, {"vin":[],"vout":[]}, {"vin":None,"vout":None}, {"vin":[None,1],"vout":[True,"secret"]}):
        raw(f"empty-{len(cases)}",raw=empty)
    base = {"version":2,"locktime":500,"vsize":100,
        "vin":[{"sequence":0xfffffffd,"prevout":{"value_sats":10000,"scriptpubkey":scripts[2]}}],
        "vout":[{"value_sats":4900,"scriptpubkey":scripts[2]},{"value_sats":4900,"scriptpubkey":scripts[4]}]}
    for algorithm in ("ecdsa", "schnorr", "unknown", None):
        for sighash in (1, 255, 256, True, None):
            observation = dict(algorithm=algorithm, sighash=sighash, low_r=True,
                low_s=False, input_index=0, verified=False, secret="discard-this-metadata")
            raw(f"adapter-{algorithm}-{sighash!r}", raw=base, signatures=[observation])
    raw("imported-signature-observations-ignored", raw={**base,
        "signature_observations": [dict(algorithm="ecdsa", sighash=1, low_r=True,
            low_s=True, input_index=0, verified=False)]})
    for field in ("version","locktime","vsize"):
        for value in (None,False,True,-2**31-1,-2**31,-1,0,1,2,3,2**31-1,2**31,2**32-1,2**32,1.0,"2"):
            doc=deepcopy(base); doc[field]=value
            raw(f"{field}-{value!r}",raw=doc)
    for version in (None,-1,1,2,3,2**31,2**32-1):
        for value in (None,False,"1",-1,0,1,0xffff,1<<22,(1<<22)|0xffff,1<<31,0xfffffffd,0xfffffffe,0xffffffff,2**32):
            doc=deepcopy(base); doc["version"]=version; doc["vin"][0]["sequence"]=value
            raw(f"sequence-{version}-{value!r}",raw=doc)
    for field in ("value","value_sats"):
        for value in (None,False,True,-1,0,1,1.9,1e20,1e100,0.000000015,0.000000025,2**100,"1","1.9","1e-8","1.0e-8"," 1_000 ","١٠٠٠","-0.000000015","NaN",{},[],""):
            doc=deepcopy(base); doc["vout"]=[dict(scriptpubkey=scripts[2],**{field:value})]
            raw(f"amount-{field}-{value!r}",raw=doc)
    for field in ("scriptpubkey","script_hex","scriptPubKey"):
        for value in (scripts[2],{"hex":scripts[2]},{"hex":"","scriptpubkey":scripts[2]},None,True,{}):
            doc=deepcopy(base); doc["vout"][0]={"value_sats":1000,field:value}
            raw(f"script-shape-{field}-{value!r}",raw=doc)
    pub = b"\2"+b"\x11"*32
    redeem = b"\x51"+push(pub)+b"\x51\xae"
    p2pkh = b"\x76\xa9\x14"+h160(pub)+b"\x88\xac"
    p2wpkh = b"\0\x14"+h160(pub)
    p2wsh = b"\0\x20"+sha256(redeem).digest()
    structures = [
        ("p2pkh",p2pkh,push(sig)+push(pub),[]),
        ("p2pk",push(pub)+b"\xac",push(sig),[]),
        ("p2sh-multisig",b"\xa9\x14"+h160(redeem)+b"\x87",b"\0"+push(sig)+push(redeem),[]),
        ("p2wpkh",p2wpkh,b"",[sig,pub]),
        ("p2wsh",p2wsh,b"",[b"",sig,redeem]),
        ("nested-wpkh",b"\xa9\x14"+h160(p2wpkh)+b"\x87",push(p2wpkh),[sig,pub]),
        ("nested-wsh",b"\xa9\x14"+h160(p2wsh)+b"\x87",push(p2wsh),[b"",sig,redeem]),
        ("taproot",bytes.fromhex(scripts[4]),b"",[b"\x11"*64]),
        ("taproot-annex",bytes.fromhex(scripts[4]),b"",[b"\x11"*64,b"\x50annex-secret"]),
        ("arbitrary-data",b"\x51",push(sig),[sig,pub]),
    ]
    for name, script, scriptsig, witness in structures:
        doc=deepcopy(base)
        doc["vin"][0].update(prevout={"value_sats":10000,"scriptpubkey":script.hex()},scriptsig=scriptsig.hex(),witness=[v.hex() for v in witness])
        raw(name,raw=doc,source="psbt_proposal",subject="opaque:proposal")
        wrong=deepcopy(doc)
        wrong_script=bytearray(script);wrong_script[min(10,len(script)-1)]^=1
        wrong["vin"][0]["prevout"]["scriptpubkey"]=wrong_script.hex()
        raw(name+"-wrong-binding",raw=wrong)
        for key in ("witness","scriptsig"):
            broken=deepcopy(doc); broken["vin"][0][key]=["xx"] if key=="witness" else "4dff"
            raw(name+"-malformed-"+key,raw=broken)
        if scriptsig:
            for width in (1,2,4):
                d=deepcopy(doc)
                d["vin"][0]["scriptsig"]=b"".join(push(v,width) for v in ([sig,pub] if name=="p2pkh" else [sig] if name=="p2pk" else [b"",sig,redeem] if name=="p2sh-multisig" else [p2wpkh if name=="nested-wpkh" else p2wsh])).hex()
                raw(name+f"-push-width-{width}",raw=d)
    rng=random.Random(21)
    for i in range(80):
        doc={"version":rng.choice((1,2,3,0xffffffff)),"locktime":rng.choice((0,1,500000000)),"vsize":rng.choice((0,100,200)),
            "vin":[{"sequence":rng.choice((0,2,0xfffffffd,0xffffffff)),"prevout":{"value_sats":rng.randint(0,10000),"scriptpubkey":rng.choice(scripts[:9])}} for _ in range(rng.randrange(5))],
            "vout":[{"value_sats":rng.choice((0,1000,2000,-1)),"scriptpubkey":rng.choice(scripts[:9])} for _ in range(rng.randrange(6))]}
        raw(f"random-{i}",raw=doc)
    snapshot=extract_transaction_features(base,source="untrusted-source",subject_id="private-subject")
    add("normalize","duplicate-short-cache","normalize",snapshot={**snapshot,"features":[snapshot["features"][0]]*2})
    for source, subject in (("stored_transaction",None),("psbt_proposal","safe-subject")):
        add("normalize",f"rebind-{source}","normalize",snapshot=snapshot,source=source,subject=subject)
    for key,value in (("schema_version",True),("schema_version",1.0),("extractor_version",False),("schema_version",2),("features",[]),("features",None),("features",[None]),("features",snapshot["features"]*2)):
        doc=deepcopy(snapshot);doc[key]=value
        add("normalize",f"root-{key}-{value!r}"[:100],"normalize",snapshot=doc)
    for index,row in enumerate(snapshot["features"]):
        for value in (None,True,False,1,-1,1.0,"secret",[],{},dict(row["value"], secret="secret") if isinstance(row["value"],dict) else ["secret"]):
            doc=deepcopy(snapshot);doc["features"][index]["value"]=value
            add("normalize",f"value-{row['code']}-{value!r}"[:100],"normalize",snapshot=doc)
        for field,value in (("code","secret"),("availability","secret"),("source","secret"),("evidence",[{"private":"secret"}]),("assumptions",["secret"]),("confidence","secret")):
            doc=deepcopy(snapshot);doc["features"][index][field]=value
            add("normalize",f"metadata-{row['code']}-{field}","normalize",snapshot=doc)
        for availability in ("observed","partial","unavailable"):
            doc={**snapshot,"features":[{**row,"availability":availability}]}
            add("normalize",f"availability-{row['code']}-{availability}","normalize",snapshot=doc)
    for path in (("cardinality","inputs"),("sequences","values"),("absolute_locktime","kind"),("signature_encodings","observations"),("witness_shapes",),("output_value_pattern","equal_groups"),("output_ordering","bip69_value_script_order"),("fee_rate","fee_sats"),("input_script_types","counts"),("output_script_types","by_index")):
        for value in (None,True,-1,4097,2**32,2**64,"secret",[None]*4097):
            doc=deepcopy(snapshot)
            row=next(r for r in doc["features"] if r["code"]==path[0])
            if len(path)==1: row["value"]=value
            else: row["value"][path[1]]=value
            add("normalize",f"bounds-{path}-{str(value)[:20]}","normalize",snapshot=doc)
    rich = deepcopy(base)
    rich.update(version=3,locktime=500)
    rich["vin"] *= 3
    for row in rich["vin"]:
        row["sequence"] = 1
    rich["vout"]=[{"value_sats":v,"scriptpubkey":s} for v,s in ((0,scripts[5]),(1000,scripts[2]),(1000,scripts[4]),(2000,scripts[2]))]
    partial=[dict(algorithm="ecdsa",sighash=3,low_r=True,low_s=True,input_index=i,verified=False) for i in range(2)]
    rich_snapshot=extract_transaction_features(rich,signature_observations=partial)
    final=deepcopy(base);final["vin"][0]["sequence"]=0xffffffff
    final_snapshot=extract_transaction_features(final)
    add("rules","ineffective-locktime","evaluate",snapshot=final_snapshot)
    add("rules","ineffective-locktime-missing-sequences","evaluate",snapshot={**final_snapshot,"features":[r for r in final_snapshot["features"] if r["code"]!="sequences"]})
    for collaboration in (None,{},dict(evidence_level="reviewed")):
        add("rules",f"all-rules-{collaboration}","evaluate",snapshot=rich_snapshot,collaboration=collaboration)
    for code in [r["code"] for r in rich_snapshot["features"]]:
        for change in ("remove","unavailable","partial"):
            doc=deepcopy(rich_snapshot)
            if change=="remove":doc["features"]=[r for r in doc["features"] if r["code"]!=code]
            else:next(r for r in doc["features"] if r["code"]==code)["availability"]=change
            add("rules",f"prerequisite-{code}-{change}","evaluate",snapshot=doc)
    for shape in ((3,1),(3,4),(3,0),(0,4),(2,1)):
        for collab in (None,{}):
            doc=deepcopy(base);doc["vin"]*=shape[0];doc["vout"]=[{"value_sats":v+1000,"scriptpubkey":scripts[2]} for v in range(shape[1])]
            add("rules",f"shape-{shape}-{collab}","evaluate",snapshot=extract_transaction_features(doc),collaboration=collab)
    for fee,size in ((100,100),(99,100),(101,100),(1001,1000),(1009,1000),(1010,1000),(0,100),(100,0),(10**100,100)):
        for collab in (None,{}):
            doc=deepcopy(snapshot);row=next(r for r in doc["features"] if r["code"]=="fee_rate");row.update(value={"fee_sats":fee,"vsize":size},availability="observed")
            add("rules",f"fee-{fee}-{size}-{collab}","evaluate",snapshot=doc,collaboration=collab)
    return cases


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check",action="store_true")
    args=parser.parse_args()
    cases=vectors()
    text=json.dumps(cases,ensure_ascii=True,sort_keys=True,separators=(",",":"))+"\n"
    if args.check:
        assert DEST.read_text()==text, "public-feature vectors drifted"
    else:
        DEST.parent.mkdir(parents=True,exist_ok=True)
        DEST.write_text(text)
    print(f"{len(cases)} public-feature vectors")


if __name__=="__main__":
    main()
