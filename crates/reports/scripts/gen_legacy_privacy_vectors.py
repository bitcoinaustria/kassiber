import argparse
from contextlib import closing
import hashlib
import json
from pathlib import Path
import sqlite3
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber.core import privacy_hygiene as reference
from kassiber.core.chain_analysis.features import extract_transaction_features
from kassiber.errors import AppError
from tests.coinjoin_shape_vectors import vectors as coinjoin_shapes

ROOT = Path(__file__).resolve().parents[3]
DEST = ROOT / "crates/reports/tests/data/legacy_privacy.json"
METADATA = ROOT / "crates/reports/src/legacy_privacy_metadata.json"
SEED = ROOT / "crates/reports/tests/data/privacy_hygiene_seed.sql"
# Books built from the frozen seed keep the tables it was cut from.
SEED_TABLE_COUNT = sum(line.startswith("CREATE TABLE ") for line in SEED.read_text().splitlines())
BOOK = SEED.with_name("book.sql")
SCRIPTS = ["0014" + "11" * 20, "76a914" + "22" * 20 + "88ac", "5120" + "33" * 32, "0020" + "44" * 32, "a914" + "55" * 20 + "87", "6a0141", "zz", None]
TXID = "ab" * 32


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def tables(conn):
    names = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert len(names) == SEED_TABLE_COUNT
    return {name: sorted([dict(r) for r in conn.execute(f'SELECT * FROM "{name}"')], key=canonical) for name in names}


def connection(sql=""):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    conn.executescript(SEED.read_text() + BOOK.read_text() + sql)
    return conn


def quote(value):
    if value is None:
        return "NULL"
    if isinstance(value, (int, float)):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


def insert(table, **values):
    return f"INSERT INTO {table}({','.join(values)}) VALUES({','.join(map(quote,values.values()))});"


def tx(raw=None, *, id="legacy", wallet="cold", boundary=None, direction="outbound", excluded=0, external=TXID, asset="BTC", review=None, date="2025-01-01", fee=1000000):
    if raw is None:
        raw = graph()
    return insert("transactions", id=id, workspace_id="ws", profile_id="p", wallet_id=wallet, fingerprint=id,
        external_id=external, asset=asset, direction=direction, amount=1000000000, fee=fee, raw_json=json.dumps(raw),
        privacy_boundary=boundary, review_status=review, excluded=excluded, occurred_at=date, created_at="fixed")


def coin(n=0, *, wallet="cold", script=SCRIPTS[0], address=None, amount=100000000, spent=None, anonymity=None, txid=TXID, network="main", chain="bitcoin", vout=None):
    return insert("wallet_utxos", id=f"coin-{wallet}-{n}", workspace_id="ws", profile_id="p", wallet_id=wallet,
        chain=chain, network=network, asset="LBTC" if chain == "liquid" else "BTC", amount=amount,
        txid=txid, vout=n if vout is None else vout, outpoint=f"{txid}:{n}", confirmation_status="confirmed", confirmations=5,
        script_pubkey=script, address=address, spent_at=spent, anonymity_score=anonymity, first_seen_at="fixed", last_seen_at="fixed")


def graph(inputs=2, outputs=2, *, values=None, input_value=123457, scripts=None, version=2, sequence=0xffffffff, **extra):
    values = values if values is not None else [12349 + i for i in range(outputs)]
    scripts = scripts if scripts is not None else [SCRIPTS[0]] * outputs
    return dict(txid=TXID, version=version, locktime=0, vsize=140,
        vin=[dict(txid=f"{i+1:064x}", vout=0, sequence=sequence, prevout=dict(value_sats=input_value, scriptpubkey=SCRIPTS[0])) for i in range(inputs)],
        vout=[dict(n=i, value_sats=value, scriptpubkey=script) for i,(value,script) in enumerate(zip(values,scripts))], **extra)


def evaluate(conn, args):
    try:
        payload = reference.build_privacy_hygiene_snapshot(conn,args)
        return {"ok":payload}
    except AppError as exc:
        return {"error":dict(code=exc.code,message=str(exc),hint=exc.hint,details=exc.details,retryable=exc.retryable)}
    except (ValueError,OverflowError) as exc:
        return {"error":dict(code="internal_error",message=str(exc),hint=None,details=None,retryable=False)}


def probes(conn):
    wallets = reference._load_wallets(conn,"p",wallet_id=None)
    inventory = reference._load_inventory(conn,"p",wallet_id=None)
    index = reference._inventory_indexes(inventory,{w["id"]:w for w in wallets})
    rows = reference._load_transactions(conn,"p",wallet_id=None,transaction_ref=None,include_excluded=True)
    context = reference._collaboration_context(rows)
    results = []
    for row in rows:
        scored = reference._score_transaction(row,index,context)
        results.append(dict(id=row["id"], **{key:scored[key] for key in ("score","state","findings")}))
    return results


def vectors():
    result = []
    base = "DELETE FROM transactions; DELETE FROM wallet_utxos;"

    def add(name, group="transaction", *, raw=None, sql=None, args=None, **kwargs):
        sql = base + (tx(raw,**kwargs) if sql is None else sql)
        with closing(connection(sql)) as conn:
            before = tables(conn)
            expected = evaluate(conn,args)
            result.append(dict(name=name,group=group,sql=sql,args=args,args_json=json.dumps(args),expected=expected,probes=probes(conn),
                table_sha256={name:hashlib.sha256(canonical(rows).encode()).hexdigest() for name,rows in before.items()}))
            assert tables(conn) == before

    for n in (0,1,2,4,5,9,10,19,20,49,50,51):
        add(f"common-input-{n}",raw=graph(inputs=n))
    for n in (0,1,2,3,4,5):
        add(f"round-count-{n}",raw=graph(values=[100000]*n + [12349],scripts=[SCRIPTS[0]]*(n+1)))
    for n in (0,1,9999,10000,99999,100000,200000,500000,1000000,2000000,5000000,10000000,20000000,25000000,50000000,100000000,200000000,500000000,1000000000,12345678):
        add(f"round-value-{n}",raw=graph(values=[n,12349]))
    for n in (1,2,3,5,10,15,20,25,50,100,4,99):
        for delta in (-2,-1,0,1,2):
            add(f"rate-{n}-{delta}",raw=graph(fee_sats=140*n+delta))
    for sequence in (None,False,-1,0,1,0x7fffffff,0xfffffffd,0xfffffffe,0xffffffff,0x100000000):
        add(f"sequence-{sequence}",raw=graph(sequence=sequence))
    for values in ([200000,300000],[123456,123457],[123457,123458],[1,2],[None,12349]):
        add(f"unnecessary-{values}",raw=graph(values=values))
    for a in range(len(SCRIPTS)):
        for b in range(len(SCRIPTS)):
            add(f"types-{a}-{b}",raw=graph(scripts=[SCRIPTS[a],SCRIPTS[b]]))
    for count in (1,2,3,4):
        for tr in range(count+1):
            add(f"taproot-{count}-{tr}",raw=graph(values=list(range(12341,12341+count)),scripts=[SCRIPTS[2]]*tr+[SCRIPTS[0]]*(count-tr)))
    for position in (0,1,2,3):
        add(f"owned-position-{position}",sql=tx(graph(outputs=4)) + coin(position,script=None))
    for direction in ("inbound"," INBOUND ","outbound","other",""):
        add(f"attribution-{direction}",raw=graph(scripts=[SCRIPTS[1],SCRIPTS[5]],values=[100000,0],sequence=1,fee_sats=140),direction=direction)
    add("score-clamp-zero",raw=graph(inputs=50,values=[100000,200000,300000,0],scripts=[SCRIPTS[0],SCRIPTS[1],SCRIPTS[0],SCRIPTS[5]],sequence=1,fee_sats=140))
    upper = graph(inputs=5,values=[100000]*5,scripts=[SCRIPTS[2]]*5,fee_sats=0)
    for vin in upper["vin"]:
        vin["prevout"]["scriptpubkey"] = SCRIPTS[2]
    add("score-clamp-hundred","collaboration",raw=dict(upper,islikelycoinjoin=True),boundary="payjoin")
    for raw in ({},[],None,{"vin":[],"vout":[]},{"vin":[1,None],"vout":[True]}, {"vin":[{},None],"vout":[{},0]}):
        add("coverage-" + canonical(raw),"scope",raw=raw if raw is not None else "invalid JSON")
    for raw in (graph(),dict(tx=graph()),dict(chain="liquid",network="main",tx=graph()),dict(chain="future",tx=graph()),dict(chain="lightning",tx=graph())):
        add("shape-" + canonical(raw),"scope",raw=raw)
    for args in ({"wallet":"cold"},{"wallet":"Cold"},{"wallet":"missing"},{"wallet":True},{"transaction":"legacy"},{"transaction":TXID.upper()},{"transaction":"missing"},{"unknown":1},[],True,3,"args"):
        add("args-" + canonical(args),"scope",args=args)
    for key in ("wallet","transaction"):
        for value in ({"z":2,"a":1},[2,1],False,0,1.5):
            add("reference-coercion-" + key + canonical(value),"scope",args={key:value})
    for limit in (None,"",0,-1,False,True,1,2,50,200,201,1.9,"2.0"," 2 ","٢","1_0",[],{},10**80):
        add("limit-" + canonical(limit),"scope",args={"limit":limit})
    add("empty-profile","scope",sql="UPDATE settings SET value='' WHERE key='context_profile';",args={"limit":0})
    add("empty-wallets","scope",sql="DELETE FROM wallets;")
    add("empty-book","scope",sql="")
    for n in (0,1,2,3,4,5,6,10,19,20,21,49,50,51):
        add(f"inventory-{n}","inventory",sql="".join(coin(i,address="bc1q-reused" if i%2 else "1-reused",amount=999999 if i%3 else 1000000,script=SCRIPTS[i%5],anonymity=20 if i%2 else 19) for i in range(n)))
    for groups in range(1,7):
        add(f"reuse-groups-{groups}","inventory",sql="".join(coin(i,address=f"bc1q-reused-{i%groups}") for i in range(groups*2)))
    for script in SCRIPTS:
        for address in (None,"bc1p-test","tb1q-test","bcrt1q-test","m-test","2-test","ex1-test","unknown",""):
            add(f"address-{script}-{address}","inventory",sql=coin(address=address,script=script))
    for n in (0,1,19,20,21,99):
        add(f"anonymity-{n}","inventory",sql="".join(coin(i,anonymity=n) for i in range(7)))
    for amount in (-1001,-1,0,1,999000,999999,1000000,1000001,9223372036854775807):
        for spent in (None,"", " ","fixed"):
            add(f"dust-{amount}-{spent}","inventory",sql=coin(amount=amount,spent=spent))
    for amount in ("bad","1.5"):
        for spent in (None,"fixed"):
            add(f"invalid-amount-{amount}-{spent}","inventory",sql=coin(amount=amount,spent=spent))
    for name,sql in (
        ("same-outpoint",coin() + coin(1,wallet="hot",vout=0)),
        ("same-script",coin() + coin(1,wallet="hot")),
        ("same-script-same-wallet",coin() + coin(1)),
        ("different-network",coin(network="test")),
        ("different-chain",coin(chain="liquid")),
        ("invalid-domain",coin(network="future")),
        ("invalid-script",coin(script="xx")),
        ("missing-input-value",coin(txid="0"*63+"1")),
    ):
        add(name,"inventory",sql=tx(graph(input_value=None)) + sql)
    add("same-outpoint-known-values","inventory",sql=tx(graph()) + coin() + coin(1,wallet="hot",vout=0))
    for boundary in ("coinjoin","payjoin","collaborative","payment-in-coinjoin","sweep","invalid","", " COINJOIN "):
        for review in (None,"accepted","reviewed"," REVIEWED "):
            add(f"boundary-{boundary}-{review}","collaboration",boundary=boundary,review=review)
    for key in ("privacy_boundary","privacy_boundary_kind","privacyBoundary","privacyBoundaryKind","privacy_hop","privacy_hop_kind","privacyHop","privacyHopKind","islikelycoinjoin","is_likely_coinjoin","isLikelyCoinJoin","likely_coinjoin","likelyCoinJoin"):
        for value in ("coinjoin","bad",True,"yes",False,None):
            add(f"raw-{key}-{value}","collaboration",raw=dict(graph(),**{key:value}))
    for inputs,outputs in ((4,5),(5,4),(5,5),(5,12),(5,13),(19,20),(20,19),(20,20),(50,20)):
        add(f"equal-{inputs}-{outputs}","collaboration",raw=graph(inputs=inputs,values=[100000]*outputs,scripts=[SCRIPTS[0]]*outputs))
    for shape in coinjoin_shapes():
        add(f"coinjoin-shape-{shape['name']}","collaboration",raw=dict(shape["raw"],txid=TXID))
    unvalued = next(shape["raw"] for shape in coinjoin_shapes() if shape["name"] == "wabisabi_without_input_values")
    owned = next(shape["raw"] for shape in coinjoin_shapes() if shape["name"] == "wabisabi_four_per_denomination")
    add("coinjoin-shape-owned-input-amounts","collaboration",sql=tx(dict(unvalued,txid=TXID)) + "".join(
        coin(i,vout=entry["vout"],txid=entry["txid"],amount=known["prevout"]["value"],script=known["prevout"]["scriptpubkey"]) for i,(entry,known) in enumerate(zip(unvalued["vin"],owned["vin"]))))
    for extra in ({"privacy_boundary":"bad"},{"raw_json":{"privacyHop":"payjoin"}},{"raw_json":json.dumps({"privacyHop":"payjoin"})}):
        add("nested-boundary-"+canonical(extra),"collaboration",raw=dict(graph(),**extra))
    for excluded in (0,1):
        for network in ("main","test"):
            for external in (TXID,"cd"*32):
                add(f"other-marker-{excluded}-{network}-{external}","collaboration",args={"wallet":"cold"},sql=tx() + tx(dict(graph(),network=network),id="marker",wallet="hot",boundary="payjoin",excluded=excluded,external=external))
    for version in (-1,0,1,2,3,4,0x7fffffff):
        for seq in (1,0xffffffff):
            raw = graph(version=version,sequence=seq)
            raw["locktime"] = 123456
            add(f"structural-{version}-{seq}","structure",raw=raw)
    features = extract_transaction_features(graph())
    for valid in (True,False):
        cached = json.loads(json.dumps(features))
        cached["features"] = [dict(f,value=4) if f["code"] == "transaction_version" else f for f in cached["features"]]
        if not valid:
            cached["features"][0]["secret"] = "private-cache-payload"
        add(f"cached-features-{valid}","structure",raw=dict(graph(),chain_analysis_features=cached))
    signatures = dict(schema_version=1,extractor_version=1,features=[dict(code="signature_encodings",value={"count":2,"observations":[dict(algorithm="ecdsa",sighash=2,low_r=True,low_s=True,input_index=n,verified=False) for n in range(2)]},availability="observed")])
    add("cached-low-r-and-sighash","structure",raw=dict(graph(),chain_analysis_features=signatures))
    for n in (0,1,2,3,10,50,201):
        add(f"ordering-limit-{n}","ordering",args={"limit":2},sql="".join(tx(graph(inputs=i%6,sequence=1 if i%2 else 0xffffffff),id=f"row-{i:03d}",date=f"2025-01-{i%28+1:02d}",direction="inbound" if i%4 == 0 else "outbound") for i in range(n)))
    for value in (None,True,False,0,-1,1.0,0.00001,"1", "0.00001","1e2","١_٠", "invalid",10**80):
        raw = graph()
        raw["vout"][0] = {"value":value,"scriptPubKey":{"hex":SCRIPTS[0],"addresses":["bc1q-test"]}}
        add("value-" + canonical(value),"coercion",raw=raw)
        add("fee-" + canonical(value),"coercion",raw=dict(graph(),fee_sats=value))
    return result


def metadata():
    return dict(reference.FINDING_METADATA,inbound_counterparty=sorted(reference.INBOUND_COUNTERPARTY_FINDINGS))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check",action="store_true")
    args = parser.parse_args()
    cases = vectors()
    if args.check:
        assert json.loads(DEST.read_text()) == cases
        assert json.loads(METADATA.read_text()) == metadata()
    else:
        DEST.write_text(json.dumps(cases,indent=2,sort_keys=True) + "\n")
        METADATA.write_text(json.dumps(metadata(),indent=2,sort_keys=True) + "\n")
    print(f"{len(cases)} legacy privacy vectors")


if __name__ == "__main__":
    main()
