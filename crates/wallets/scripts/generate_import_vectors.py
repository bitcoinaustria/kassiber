#!/usr/bin/env python3
"""Freeze Python file-reader/normalization behavior, never a candidate oracle."""
import argparse
import json
import math
import tempfile
from pathlib import Path
from kassiber.importers import load_import_records
from kassiber.core.imports import normalize_import_record
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.msat import btc_to_msat
from kassiber.errors import AppError

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_imports.json"
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description"
CASES = {
    "plain": HEADER + "\n2025-01-02,abc,in,BTC,1.000000000005,-0.000000000005,60000,receipt\n",
    "quotes": HEADER + '\r\n2025-01-02,abc,out,btc,-0.001,0.00000000001,,"hello, ""world""\r\nnext"\r',
    "blank-short": HEADER + "\n\n2025-01-02,a,in,BTC,1\n",
    "quote-tail": HEADER + '\n2025-01-02,a,in,BTC,1,0,,"x"tail\n',
    "unclosed": HEADER + '\n2025-01-02,a,in,BTC,1,0,,"x\n',
    "bom": "\ufeff" + HEADER + "\n2025-01-02,a,in,BTC,1,0,,x\n",
    "duplicates": "date,date,amount\nwrong,2025-01-02,2\n",
    "extra": HEADER + "\n2025-01-02,a,in,BTC,1,0,,x,extra\n",
    "empty": "",
    "blank-header": "\n2025-01-02,1\n",
    "direction": "date,direction,amount\n2025-01-02, bad ,2\n",
    "decimal": "date,amount\n2025-01-02,junk\n",
    "rate-space": "date,amount,fiat_rate\n2025-01-02,1, \n",
    "blank-date": "date,amount\n,1\n",
    "precision": "date,amount,fee,fiat_rate\n2025-01-02,1.23456789012345678901234567890123456789,0.0000000000049999999999999999999999999,1234.1234567890123456789012345678901234\n",
    "tiny": "date,amount,fee\n2025-01-02,-0.0000000000049,0.000000000005\n",
    "unicode-decimal": "date,amount,fee\n2025-01-02, １２.３４ ,١_٢e-٣\n",
    "raw-provenance": 'date,txid,amount,raw_json\n2025-01-02,abc,1,"{""_kassiber_provenance"":{},""source"":""user""}"\n',
    "native-txid": "date,txid,amount\n2025-01-02," + "A"*64 + ",1\n",
    "utf8-bad": b"date,amount\n2025-01-02,\xff\n",
    "utf16": (HEADER + "\n").encode("utf-16"),
    "field-limit": "description\n" + "x"*131073 + "\n",
}
for stamp in ["2025-01-02T00:00Z", "2025-01-02T00:00 Z", "2025-01-02T00:00 +01:00", "2025-01-02T00:00:01.123456Z", "2025-01-02T00:00:Z", "20250102T000000", "2025-W01-4", "2025-01-02T00:00:00z", "2025-01-02T00:00:00+00:99", "2025-01-02T00:00:00 +01:00"]:
    CASES["time-"+stamp] = f"date,amount\n{stamp},1\n"

for key in ("amount","fee","fiat_rate","fiat_value"):
    for value in ("NaN","-NaN123","sNaN","Infinity","-Infinity"," ","\x1c1\x1c"):
        CASES[f"number-{key}-{value}"] = f"date,amount,direction,{key}\n2025-01-02,1,in,{value}\n"

def clean(value):
    if isinstance(value,float) and math.isnan(value): return None
    if isinstance(value,float) and not math.isfinite(value): return {"$float":str(value)}
    if isinstance(value,dict): return {k:clean(v) for k,v in value.items()}
    if isinstance(value,list): return [clean(v) for v in value]
    return value

def vectors():
    result=[]
    with tempfile.TemporaryDirectory(prefix="kassiber-import-vectors-") as root:
        file=Path(root)/"input.csv"
        for name, content in CASES.items():
            data=content.encode() if isinstance(content,str) else content
            file.write_bytes(data)
            case={"name":name,**({"text":content} if isinstance(content,str) else {"bytes":list(data)})}
            try:
                records=[]
                for row in load_import_records(str(file),"csv"):
                    r=normalize_import_record(row,"file:csv")
                    fp=make_transaction_fingerprint("wallet",r["external_id"],r["occurred_at"],r["direction"],r["asset"],r["amount"],r["fee"])
                    r["amount"]=btc_to_msat(r["amount"])
                    r["fee"]=btc_to_msat(r["fee"])
                    for key in ["exchange_evidence_match_by_economics","exchange_evidence_match_time_tolerance_seconds","match_existing_ignore_fee"]:
                        r.pop(key)
                    r["amount_includes_fee"]=int(r["amount_includes_fee"])
                    records.append({"fields":r,"fingerprint":fp})
                case["records"]=clean(records)
            except Exception as exc:
                case["error"]={"code":exc.code if isinstance(exc,AppError) else "internal_error","message":str(exc),"hint":exc.hint if isinstance(exc,AppError) else None}
            result.append(case)
    return result

if __name__=="__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("--check",action="store_true")
    args=parser.parse_args()
    value=json.dumps(vectors(),ensure_ascii=True,indent=2)+"\n"
    if args.check:
        if DEST.read_text()!=value: raise SystemExit("Import vectors drifted")
    else: DEST.write_text(value)
