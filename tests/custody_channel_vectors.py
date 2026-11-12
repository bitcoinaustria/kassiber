"""CPython lifecycle branches, outside the raw-only snapshot input."""
from copy import deepcopy
import json
from pathlib import Path

from kassiber.core.lightning import channel_lifecycle as ref
from kassiber.core.custody_journal import channel_context_from_records
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_channel_helpers.json"


def evaluate(op, value):
    if op == "outpoint": return ref._canonical_outpoint(value)
    if op == "txid": return ref._txid_from_outpoint(value)
    if op == "tx_scope": return ref._transaction_lifecycle_scope(value)
    if op == "funding_scope": return ref._funding_lifecycle_scope(value)
    if op == "closing_scope": return ref._closing_lifecycle_scope(value)
    if op == "vin": return sorted(ref._vin_outpoints(value))
    if op == "sweeps": return sorted(ref._trusted_local_close_sweeps(value))
    if op == "funding_mismatch": return ref._funding_amount_mismatch(value["row"], value["funded"], strict=value.get("strict",False))
    if op == "close_mismatch": return ref._close_balance_mismatch(value["received"],value["balance"])
    if op == "roles": return ref.channel_role_map(value["channels"],value["rows"])
    if op == "pairs": return ref.channel_transfer_pairs(value["channels"],value["rows"],value["wallets"])
    if op == "context":
        roles,pairs=channel_context_from_records(value["records"],value["rows"],value["wallets"])
        return dict(roles=roles,pairs=pairs)
    raise AssertionError(op)


def vectors():
    vectors=[]
    def add(op,case,value):
        try: result=dict(expected=evaluate(op,value))
        except Exception as exc: result=dict(error=type(exc).__name__)
        vectors.append(to_jsonable(dict(op=op,case=case,value=value,**result),redact_config=False))
    txid="a"*64
    for index,value in enumerate((None,"",False,[],{},txid,txid.upper()+":+١_٢",txid+":-1",txid+":0",txid+":1.5",txid+":", "bad:0",txid+":"+str(2**160),txid+":"+"9"*4301, "\u001c"+txid+":1\u001c")):
        add("outpoint",f"point_{index}",value); add("txid",f"txid_{index}",value)
    scope=dict(id="row",external_id=txid,external_id_kind="txid",config_json='{"chain":"bitcoin","network":"main"}',asset="BTC")
    for i,changes in enumerate(({},dict(asset=""),dict(asset="LBTC"),dict(asset="USD"),dict(config_json='{"chain":"liquid","network":"liquidv1"}'),dict(raw_json=json.dumps(dict(txid="b"*64))),dict(external_id="bad"),dict(id="owned-derive:row"),dict(direction="wrong"))):
        add("tx_scope",f"tx_scope_{i}",scope|changes)
    channel=dict(funding_txid=txid,funding_outpoint=txid+":0",closing_txid="b"*64,config_json=scope["config_json"],wallet_id="node",channel_id="legacy",funding_amount_msat=500_000_000,close_balance_msat=500_000_000)
    for i,changes in enumerate(({},dict(funding_txid=""),dict(funding_outpoint=""),dict(funding_txid="bad"),dict(funding_outpoint="bad:0"),dict(funding_outpoint="b"*64+":0"),dict(funding_txid="",funding_outpoint=""),dict(chain="liquid"),dict(network="regtest"),dict(config_json=None,chain="bitcoin",raw_json='{"network":"main"}'),dict(config_json=None,wallet_config_json='{"chain":"bitcoin","network":"regtest"}'),dict(raw_json='{"network":"regtest"}'),dict(raw_json='{"network":Infinity}'),dict(raw_json='{"network":"MAIN"}'),dict(raw_json=[]),dict(raw_json="[1]"),dict(raw_json="not json"),dict(closing_txid=None),dict(closing_txid="bad"),dict(closing_txid="B"*64),dict(raw_json='{"network":"\\ud800"}'))):
        add("funding_scope",f"fund_scope_{i}",channel|changes);add("closing_scope",f"close_scope_{i}",channel|changes)
    for i,raw in enumerate(('{"vin":[{"txid":"\\ud800","vout":0}]}',json.dumps(dict(vin=[dict(txid=txid,vout="\ud800")])),None,False,{},dict(vin=[dict(txid=txid,vout=1)]),"{}","[]","{",json.dumps(dict(vin=[{},False,dict(txid=txid,vout=0),dict(txid=txid,vout=-1),dict(txid=txid,vout=True),dict(txid=txid,vout="١٢") ])),json.dumps(dict(tx=dict(vin=[dict(txid=txid,vout=3)]))),json.dumps(dict(vin={},tx=[])))):
        add("vin",f"vin_{i}",dict(raw_json=raw))
    good=dict(_kassiber_provenance=dict(import_source="lnd_adapter"),channel_close_local_sweeps=[dict(sweep_txid=txid,outpoint="b"*64+":0")])
    for i,raw in enumerate((good,{},good|dict(_kassiber_provenance=[]),good|dict(_kassiber_provenance=dict(import_source="csv")),good|dict(channel_close_local_sweeps={}),good|dict(channel_close_local_sweeps=[None,{},dict(sweep_txid=3,outpoint=txid+":1"),dict(sweep_txid=txid,outpoint="bad"),dict(sweep_txid="bad",outpoint=txid+":0")]))):
        add("sweeps",f"sweeps_{i}",dict(raw_json=json.dumps(raw)))
    for funded in (-1,0,100_000_000,500_000_000,2**160,10**310):
        for recorded in (None,"bad",[],{},100_000_000,102_500_000,102_500_001,500_000_000,505_000_000,505_000_001,499_000_000,2**160):
            for strict in (False,True):
                add("funding_mismatch",f"funding_{funded}_{recorded}_{strict}",dict(row=dict(amount=recorded),funded=funded,strict=strict))
    for balance in (-1,0,500_000_000,2**160,10**310):
        for received in (0,500_000_000,495_000_000,494_999_999,505_000_000,505_000_001,2**160):
            add("close_mismatch",f"close_{balance}_{received}",dict(received=received,balance=balance))
    for i,source in enumerate(([],{})):
        add("sweeps",f"unhashable_provenance_{i}",dict(raw_json=json.dumps(good|dict(_kassiber_provenance=dict(import_source=source)))))
    bad_sweep=deepcopy(good);bad_sweep["channel_close_local_sweeps"][0]["outpoint"]="\ud800"
    add("sweeps","surrogate_outpoint",dict(raw_json=json.dumps(bad_sweep)))
    # Direct producer operands cover domains storage never emits (negative
    # amounts, missing columns, duplicate node refs, and a combined lifecycle row).
    rows=[scope|dict(id="fund",direction="outbound",amount=500_000_000,fee=100_000),scope|dict(id="close",external_id="b"*64,direction="inbound",amount=499_000_000,fee=0)]
    wallets={"node":dict(id="node",label="Node")}
    for i,changes in enumerate(({},dict(wallet_id=""),dict(wallet_id="missing"),dict(channel_id="lnd:strict"),dict(channel_id="coreln:strict"),dict(channel_id="coreln:strict",funding_amount_msat=0),dict(channel_id="coreln:strict",funding_amount_msat=500_000_001),dict(channel_id="lnd:strict",funding_amount_msat=0),dict(channel_id="lnd:strict",close_balance_msat=0),dict(funding_amount_msat="bad",close_balance_msat=[]),dict(funding_amount_msat={"nonempty":1},close_balance_msat="bad"),dict(funding_amount_msat=-1),dict(close_balance_msat=-1),dict(channel_id=None),dict(network="regtest"))):
        for op in ("roles","pairs"):
            add(op,f"{op}_row_{i}",dict(channels=[channel|changes],rows=rows,wallets=wallets))
    for i,(channels,changes) in enumerate((([channel,channel|dict(wallet_id="other")],{}),([channel|dict(channel_id="lnd:strict"),channel|dict(channel_id="lnd:strict",funding_amount_msat=0)],{}),([channel],dict(wallets={"node":{}})),([channel],dict(rows=[rows[1]])),([channel],dict(rows=[rows[0]|dict(amount="bad")])),([channel],dict(rows=[rows[0]|dict(direction="inbound"),rows[1]|dict(direction="outbound")])),([channel],dict(rows=[rows[0]|dict(amount=0),rows[1]])),([channel],dict(rows=[rows[0],rows[1]|dict(amount=0)])))):
        for op in ("roles","pairs"):
            add(op,f"{op}_population_{i}",dict(channels=channels,rows=rows,wallets=wallets|dict(other=dict(id="other",label="Other")))|changes)
    for i,records in enumerate(([],[{}],[dict(txid="")],[dict(txid=txid)],[dict(txid=txid,wallet_id="node",channel_id="c",config_json=scope["config_json"],raw_json="{}",chain="bitcoin",network="main",tag="channel_close",amount_msat=500_000_000)],)):
        add("context",f"context_{i}",dict(records=records,rows=rows,wallets=wallets))
    add("context","empty_records_admission",dict(records=[],rows=[rows[0]|dict(amount={"nonempty":1})],wallets=wallets))
    add("roles","empty_atomic_admission",dict(channels=[],rows=[rows[0]|dict(amount={"nonempty":1})],wallets=wallets))
    from tests.custody_channel_generator import CHANNEL_SCENARIOS, channel_snapshot
    for i,case in enumerate(CHANNEL_SCENARIOS):
        book=channel_snapshot(i)
        add("context","raw_"+case,dict(records=book["channel_records"],rows=book["transactions"],wallets={str(w["id"]):w for w in book["wallets"]}))
        if case in {"force_multiple", "strict_lnd", "multiple_close_scopes", "network_conflict"}:
            for variant in ("no_attest", "extra_peer", "different_network", "zero_close", "owner_reappears", "reversed_rows"):
                v=deepcopy(dict(records=book["channel_records"],rows=book["transactions"],wallets={str(w["id"]):w for w in book["wallets"]}))
                if variant == "no_attest":
                    for rec in v["records"]:rec["raw_json"]="{}"
                elif variant == "extra_peer":
                    close_rec=next(rec for rec in v["records"] if rec["tag"]=="channel_close")
                    peer=deepcopy(v["rows"][-1]);peer.update(id="peer",amount=100_000,external_id="f"*64)
                    peer["raw_json"]=json.dumps(dict(txid="f"*64,chain="bitcoin",network="main",vin=[dict(txid=close_rec["txid"],vout=999)]))
                    v["rows"].append(peer)
                elif variant == "different_network":
                    for rec in v["records"]:
                        rec["config_json"]="{}"
                        if rec["tag"]=="channel_close":rec["network"]="regtest"
                    for row in v["rows"]:
                        if row["direction"]=="inbound":
                            row["config_json"]='{"chain":"bitcoin","network":"regtest"}'
                            raw=json.loads(row["raw_json"]);raw["network"]="regtest";row["raw_json"]=json.dumps(raw)
                elif variant == "zero_close":
                    for row in v["rows"]:
                        if row["direction"]=="inbound":row["amount"]=0
                elif variant == "owner_reappears":
                    for rec in deepcopy(v["records"]):rec["wallet_id"]="wallet-c";v["records"].append(rec)
                    v["records"].extend(deepcopy(book["channel_records"]))
                else:v["rows"].reverse()
                add("context","raw_"+case+"_"+variant,v)
    for value in (0,-1):
        add("pairs",f"legacy_zero_balance_{value}",dict(channels=[channel|dict(close_balance_msat=0)],rows=[rows[0],rows[1]|dict(amount=value)],wallets=wallets))
    # Role admission must fail on malformed stored amounts before the public
    # pair call's missing wallet reference could skip that record entirely.
    record=dict(txid=txid,outpoint=txid+":0",tag="channel_open",wallet_id="missing",channel_id="legacy",config_json=scope["config_json"],raw_json="{}",chain="bitcoin",network="main",amount_msat={"nonempty":1})
    add("context","role_admission_first",dict(records=[record],rows=rows,wallets=wallets))
    add("context","role_before_public_pairs",dict(records=[record|dict(wallet_id="node",amount_msat=500_000_000),record],rows=rows,wallets={"node":{}}))
    add("pairs","int_digit_limit",dict(channels=[channel],rows=[rows[0]|dict(amount="9"*4301)],wallets=wallets))
    add("roles","stored_int_digit_limit",dict(channels=[channel|dict(funding_amount_msat="9"*4301)],rows=rows,wallets=wallets))
    book=channel_snapshot(0)
    book["channel_records"][0]["outpoint"]="b"*64+":0"
    add("context","raw_outpoint_disagreement",dict(records=book["channel_records"],rows=book["transactions"],wallets={w["id"]:w for w in book["wallets"]}))
    unhashable_channel=channel|dict(raw_json=json.dumps(dict(_kassiber_provenance=dict(import_source=[]))))
    for op in ("roles","pairs"):
        add(op,op+"_unhashable_provenance",dict(channels=[unhashable_channel],rows=rows,wallets=wallets))
    add("pairs","accounts",dict(channels=[channel],rows=rows,wallets={"node":dict(id="node",label="Node",wallet_account_id="account",account_code="100",account_label="Node account")}))
    # Additional selection and stable-order branches in the full helper domain.
    for i,changes in enumerate((dict(amount=-1),dict(amount=0),dict(amount=505_000_001),dict(occurred_at=""))):
        add("pairs",f"close_leg_{i}",dict(channels=[channel],rows=[rows[0],rows[1]|changes],wallets=wallets))
    return vectors


if __name__ == "__main__":
    PATH.write_text(json.dumps(vectors(),indent=2,ensure_ascii=True)+"\n")
