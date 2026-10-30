"""Independent CPython gap-search vectors, including existing behavior pins."""
from copy import deepcopy
import base64
from datetime import datetime,timedelta,timezone
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

from kassiber.core import custody_gaps as gaps, custody_gap_reviews as reviews
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_gap_cases.json"


def row(identifier="out", *, inbound=False, amount=1_000_000, **kwargs):
    result = dict(id=identifier, profile_id="p", wallet_id="new" if inbound else "old",
                wallet_label="New" if inbound else "Old", wallet_kind="descriptor",
                direction="inbound" if inbound else "outbound", asset="BTC",
                occurred_at="2021-01-01T00:00:00Z" if inbound else "2020-01-01T00:00:00Z",
                amount=amount, fee=0, raw_json="{}")
    result.update(kwargs)
    return result


def outcome(call):
    try:
        return dict(expected=to_jsonable(call(), redact_config=False))
    except Exception as exc:
        return dict(error=getattr(exc, "code", type(exc).__name__))


def search_cases():
    # Reuse the real Python matcher tests as an independent source of inputs.
    # Their assertions run while recording; no Rust results generate a fixture.
    from tests.test_custody_gaps import CustodyGapMatcherTests
    cases = []
    original = gaps.search_custody_gap_candidates
    for name in unittest.defaultTestLoader.getTestCaseNames(CustodyGapMatcherTests):
        def capture(rows, **kwargs):
            case = dict(case=name.removeprefix("test_") + f"_{sum(c['case'].startswith(name.removeprefix('test_') + '_') for c in cases)}",
                        rows=deepcopy(rows), limits={k:v for k,v in kwargs.items() if k != "ignored_ids"}, ignored=list(kwargs.get("ignored_ids", ())))
            result = original(rows, **kwargs)
            case["expected"] = to_jsonable(result, redact_config=False)
            cases.append(case)
            return result
        with patch("tests.test_custody_gaps.search_custody_gap_candidates", capture):
            getattr(CustodyGapMatcherTests(name), name)()
    base = [row(privacy_boundary="coinjoin"), row("in", inbound=True, amount=990_000)]
    def add(case, rows=None, **limits):
        rows = deepcopy(base if rows is None else rows)
        cases.append(dict(case=case, rows=rows, limits=limits, ignored=[], **outcome(lambda: original(rows, **limits))))
    for field, values in {
        "id":[None,"",0,"\x1c out \u3000"], "profile_id":[None,""], "wallet_id":[None,""],
        "asset":[None,"", "\u019b", "\u0264", " lbtc "], "direction":[None,"invalid"," OUTBOUND "],
        "amount":[None,False,True,-1,0,1.0,"١٠٠٠٠٠٠","²","1_000_000","+1000000"," 1000000","1000000",10**100],
        "fee":[None,False,-1,"²","0001",1.0], "excluded":[True,1,1.0,"YES","off",2],
        "amount_includes_fee":[True,"On",1.0,"false"],
        "occurred_at":[None,"bad","2020-W01-3T00:00:00Z","2020-01-01🐍00:00:00.123456+01:23:45.5",
                       "0001-01-01T00:00:00+01:00","9999-12-31T23:59:59-01:00"],
        "wallet_label":[None,0,{},["A"],"Samourai"],
        "wallet_kind":["samourai","samourai-whirlpool","whirlpool","lnd"],
        "kind":["samourai_deposit","whirlpool_mix","coinjoin","income","postmix","wages"],
        "kind_override":["income","customer-payment",False,""],
        "privacy_boundary":["payjoin","whirlpool","COINJOIN","maybe"],
        "wallet_config_json":["{}","bad","[]",'{"samourai":{"role":"child","section":"postmix"}}',
                              '{"samourai":{"role":"parent","section":"badbank"}}','{"samourai":{"role":"other","section":"postmix"}}',
                              '{"network":"signet"}','{"chain":"future"}'],
    }.items():
        for index,value in enumerate(values):
            rows=deepcopy(base);rows[0][field]=value
            add(f"normalize_{field}_{index}",rows)
    for index,kind in enumerate(("buy","airdrop","hardfork","hard_fork","income","interest","lending_interest","mining","mining_reward","routing_income","staking","wages","deposit","revenue","sale","sell","exchange_buy","customer_payment")):
        rows=deepcopy(base);rows[1]["kind_override"]=kind
        add(f"external_return_{index}",rows)
    rows=deepcopy(base);rows[0].update(samourai_role="child",samourai_section="ricochet")
    add("direct_samourai_policy",rows)
    rows=deepcopy(base);rows[0]["amount_msat"]=None
    add("explicit_null_amount_msat",rows)
    rows=deepcopy(base);rows[0].update(fee=1_000_001,amount_includes_fee=True)
    add("nonpositive_principal",rows)
    for asset in ("btc","\u019b","\u0264"):
        rows=deepcopy(base)
        for item in rows:item["asset"]=asset
        add(f"canonical_asset_{ord(asset[0])}",rows)
    rows=deepcopy(base);rows[0].update(wallet_kind="samourai",kind="postmix");rows[1]["occurred_at"]="2020-01-01T00:01:00Z"
    add("maximum_score",rows)
    rows=deepcopy(base);rows[1]["amount"]=1_250_000
    add("maximum_excess",rows)
    rows=deepcopy(base);rows[1].update(occurred_at=rows[0]["occurred_at"],id="🐍")
    add("same_instant_astral_id",rows)
    add("large_same_instant_astral_id",rows,max_input_rows=1)
    rows=deepcopy(rows);rows[1]["id"]="ascii"
    add("same_instant_ascii_id",rows)
    rows=deepcopy(base);rows[0].update(occurred_at="0001-01-01T00:00:00Z");rows[1]["occurred_at"]="9999-12-31T23:59:59.999999Z"
    add("extreme_elapsed",rows)
    for minimum in (1,650_000,800_000,950_000,1_000_000):
        rows=deepcopy(base);rows[1]["amount"]=minimum
        add(f"coverage_floor_{minimum}",rows,min_coverage_ppm=minimum)
    rows=deepcopy(base);rows[0]["privacy_boundary"]=None;rows[1]["amount"]=800_000;rows[1]["wallet_id"]="old"
    add("score_below_admission",rows)
    # Exercise the indexed scan ceiling with nearest amounts before the source.
    rows=[row(f"past-{i}",inbound=True,amount=1_000_000) for i in range(40)]+deepcopy(base)
    rows[-2]["occurred_at"]="2022-01-01T00:00:00Z";rows[-1]["occurred_at"]="2023-01-01T00:00:00Z"
    add("indexed_inspection_ceiling",rows,max_input_rows=1,max_return_pool=2)
    rows=[deepcopy(base[0])]+[row(f"in-{i}",inbound=True,amount=198_000) for i in range(5)]
    add("large_aggregate_ceiling",rows,max_input_rows=1,max_aggregate_return_legs=2)
    rows=[row(f"past-{i}",inbound=True,wallet_id=f"w-{i}") for i in range(512)]
    rows += [row("source",occurred_at="2022-01-01T00:00:00Z",privacy_boundary="coinjoin"),row("future",inbound=True,occurred_at="2023-01-01T00:00:00Z")]
    add("matching_eras_inspection_ceiling",rows,max_input_rows=1,max_return_pool=2)
    rows=[row(f"past-{i}",inbound=True) for i in range(4)]
    rows += [row("source",occurred_at="2021-01-02T00:00:00Z",privacy_boundary="coinjoin"),row("future",inbound=True,occurred_at="2021-01-03T00:00:00Z")]
    add("indexed_inspection_budget",rows,max_input_rows=1,max_return_pool=2)
    rows=[row("old-source",amount=2_000_000),row("middle-source",amount=3_000_000,occurred_at="2020-02-01T00:00:00Z"),row("latest-source",occurred_at="2020-03-01T00:00:00Z"),row("future",inbound=True,amount=990_000)]
    add("ordinary_source_tail",rows,max_source_groups=2)
    rows=[deepcopy(base[0])]+[row(f"in-{i}",inbound=True,amount=490_000) for i in range(4)]
    add("return_pool_independent_from_wallet_era",rows,max_return_pool=2)
    rows=deepcopy(base);rows[0]["kind_override"]="income"
    add("capacity_external_source",rows,max_input_rows=1)
    rows=[row("z-source",amount=600_000),row("a-source",amount=400_000,wallet_id="other",wallet_label="Other"),deepcopy(base[1])]
    add("multiple_source_labels",rows)
    for when in ("1900-02-28T23:59:59Z","2000-02-29T12:34:56Z","2020-02-29T00:00:00Z","2400-03-01T00:00:00Z"):
        rows=deepcopy(base);rows[0]["occurred_at"]=when;rows[1]["occurred_at"]="2401-01-01T00:00:00Z"
        add("calendar_"+when[:10],rows)
    for seconds,microseconds in ((2**34,-1),(2**36,-4),(2**38,-16),(315537897599,-31)):
        start=datetime(1,1,1,tzinfo=timezone.utc)
        end=start+timedelta(seconds=seconds,microseconds=microseconds)
        rows=deepcopy(base);rows[0]["occurred_at"]=start.isoformat();rows[1]["occurred_at"]=end.isoformat()
        add(f"duration_rounding_{seconds}",rows)
    # Era boundary uses truncated total_seconds, including one microsecond.
    for index,when in enumerate(("2021-06-30T00:00:00Z","2021-06-30T00:00:00.999999Z","2021-06-30T00:00:01Z")):
        rows=[deepcopy(base[0]),row("first",inbound=True,amount=500_000),row("second",inbound=True,amount=490_000)]
        rows[-1]["occurred_at"]=when
        add(f"era_gap_{index}",rows,max_return_legs=1)
        add(f"large_era_gap_{index}",rows,max_return_legs=1,max_input_rows=1)
    return cases


def worklist_range_vectors(cases):
    def ranges(rows):
        found = {}
        for row in rows:
            leg = gaps._normalize_leg(row, set())
            if leg is None:
                continue
            low = max(1, (leg.principal_msat * 800_000 // 1_000_000 + 255) // 256)
            high = leg.principal_msat + leg.principal_msat * 250_000 // 1_000_000
            if leg.asset in found:
                low, high = min(low, found[leg.asset][0]), max(high, found[leg.asset][1])
            found[leg.asset] = (low, high)
        return [[asset, *bounds] for asset, bounds in sorted(found.items())]
    return [dict(rows=case['rows'], **outcome(lambda: ranges(case['rows']))) for case in cases]


def vectors():
    cases=search_cases()
    fingerprints=[]
    for case in cases:
        for candidate in case.get("expected",{}).get("candidates",()):
            c=gaps.CustodyGapCandidate(**{k:tuple(v) if isinstance(v,list) else v for k,v in candidate.items() if k!="$type"})
            fingerprints.append(dict(candidate=candidate,candidate_fingerprint=reviews.candidate_fingerprint(c),authored_claim_fingerprint=reviews.authored_claim_fingerprint(c)))
    base=[row(privacy_boundary="coinjoin"),row("in",inbound=True,amount=990_000)]
    computed=[]
    for large in (False,True):
        for ignored,accounting in (([],[]),([],['out']),(['out'],[]),(['in'],['in']),(['out'],['out'])):
            for rows in (base,[],[base[0]]):
                args=dict(large_book=large,ignored_transaction_ids=ignored,accounting_ignored_transaction_ids=accounting)
                computed.append(dict(rows=rows,**args,**outcome(lambda:gaps.compute_gap_search_result(rows,**args)[0])))
    raw_reviews=[]
    first=dict(gap_id="gap",candidate_fingerprint="fingerprint",action="dismissed",component_id=None,event_kind="review_decision",revision=1)
    for rows in ([],[first],[first,first],[first,dict(first,action="resolved")],[dict(first,event_kind="bridge_reopened")],[dict(first,event_kind=None)],[dict(first,candidate_fingerprint=None)]):
        raw_reviews.append(dict(rows=rows,expected=reviews.dismissed_fingerprints_from_rows(rows)))
    display=[]
    for case in [*cases[:12],*filter(lambda c:c["case"]=="multiple_source_labels",cases)]:
        legs=[leg for row in case["rows"] if (leg:=gaps._normalize_leg(row,set())) is not None]
        for candidate in case.get("expected",{}).get("candidates",()):
            c=gaps.CustodyGapCandidate(**{k:tuple(v) if isinstance(v,list) else v for k,v in candidate.items() if k!="$type"})
            display.append(dict(candidate=candidate,rows=case["rows"],expected=gaps._snapshot_gap(c,legs)))
    cursors=[]
    def cursor(case,text):
        expected=outcome(lambda: list(gaps._decode_projection_cursor(text)))
        if "expected" in expected:
            expected=dict(expected_json=json.dumps(expected["expected"],ensure_ascii=True),
                          expected_id_codepoints=list(map(ord,expected["expected"][2])))
        cursors.append(dict(case=case,cursor=text,**expected))
    for ordinal in (0,1,250,10**100):
        for identifier in ("cg_abc","cg_é🐍"):
            value=gaps._encode_projection_cursor((4,12),ordinal,identifier)
            cursor(f"canonical_{ordinal}_{identifier}",value)
    for index,payload in enumerate((None,[],[[],0,"x"],[[1,True],0,"x"],[[1,2],True,"x"],[[1,2],-1,"x"],[[1,2],0,None],[[1,2,3],0,"x"],[[1,2],0.0,"x"],[[1,2],0,"x"],[[1,2],0,"\ud800"])):
        text=json.dumps(payload,separators=(",",":"))
        for encoding in ("utf-8","utf-16","utf-16-be","utf-32","utf-32-be"):
            cursor(f"shape_{index}_{encoding}","cgr3."+base64.urlsafe_b64encode(text.encode(encoding)).decode().rstrip("="))
    valid=gaps._encode_projection_cursor((4,12),0,"cg_abc")
    # json.loads(bytes) uses surrogatepass. Raw surrogate pairs in UTF-8/32
    # stay two code points; UTF-16 decoding combines its valid pairs first.
    for index,text in enumerate(('[[1,2],0,"é\ud800"]','[[1,2],0,"\ud800\udc00"]','[[1,2],0,"\\\ud800"]')):
        for encoding in ("utf-8","utf-16-le","utf-16-be","utf-32-le","utf-32-be"):
            cursor(f"raw_surrogate_{index}_{encoding}","cgr3."+base64.urlsafe_b64encode(text.encode(encoding,errors="surrogatepass")).decode().rstrip("="))
    for index,payload in enumerate((b'[[1,2],0,"\xed\xa0"]',b'[[1,2],0,"\xed\xa0A"]',b'[[1,2],0,"\xc0\x80"]',b'[[1,2],0,"\xf4\x90\x80\x80"]',b'[[1,2],0,"\xff"]',b'\xff\xfe[',b'\xff\xfe\x00\x00\x00\x00\x11\x00')):
        cursor(f"invalid_byte_encoding_{index}","cgr3."+base64.urlsafe_b64encode(payload).decode().rstrip("="))
    for index,text in enumerate(("", "0", "cgr3.","wrong."+valid[5:], valid+"=",valid+"====",valid+"$",valid+"$$$$",valid[:9]+"$"+valid[9:],valid[:-1],"cgr3.🐍","cgr3.====",valid+"A")):
        cursor(f"malformed_{index}",text)
    from tests.test_custody_gaps import CustodyGapSnapshotTests
    seeded=CustodyGapSnapshotTests();seeded.setUp()
    pages=[]
    try:
        seeded.conn.execute("INSERT INTO transactions SELECT 'competing',profile_id,wallet_id,external_id,'2021-02-01T00:00:00Z',created_at,direction,asset,amount,fee,amount_includes_fee,excluded,kind,kind_override,privacy_boundary,raw_json FROM transactions WHERE id='return'")
        seeded.conn.execute("INSERT INTO transactions SELECT 'later-2023',profile_id,wallet_id,external_id,'2023-02-01T00:00:00Z',created_at,direction,asset,amount,fee,amount_includes_fee,excluded,kind,kind_override,privacy_boundary,raw_json FROM transactions WHERE id='later-disposal'")
        population=gaps.build_gap_snapshot(seeded.conn,"profile-one",limit=500)["gaps"]
        version=gaps._gap_snapshot_version(seeded.conn,"profile-one")
        first=gaps.build_gap_snapshot(seeded.conn,"profile-one",limit=1)
        for limit,cursor,gap_id in ((1,None,None),(1,"0",None),(500,None,None),(1,first["next_cursor"],None),
                                    (1,None,population[-1]["gap_id"]),(1,None,"missing"),(0,None,None),(501,None,None),
                                    (1,"bad",None),(1,first["next_cursor"],population[0]["gap_id"]),
                                    (1,gaps._encode_projection_cursor((4,1),0,population[0]["gap_id"]),None),
                                    (1,gaps._encode_projection_cursor(version,99,population[0]["gap_id"]),None),
                                    (1,gaps._encode_projection_cursor(version,0,"changed"),None)):
            expected=outcome(lambda:gaps.build_gap_snapshot(seeded.conn,"profile-one",limit=limit,cursor=cursor,gap_id=gap_id))
            if "expected" in expected:expected["expected"].pop("summary")
            pages.append(dict(version=list(version),population=population,limit=limit,cursor=cursor,gap_id=gap_id,**expected))
        result,legs=gaps.load_gap_search_result(seeded.conn,"profile-one")
        raw,_large=gaps.load_gap_search_worklist(seeded.conn,"profile-one")
        for candidate in result.candidates:
            display.append(dict(candidate=to_jsonable(candidate,redact_config=False),rows=raw,expected=gaps._snapshot_gap(candidate,legs)))
    finally:seeded.tearDown()
    return dict(search=cases,compute=computed,fingerprints=fingerprints,reviews=raw_reviews,display=display,cursors=cursors,pages=pages,
                worklist_ranges=worklist_range_vectors(cases))


STAGES_PATH=PATH.with_name("custody_gap_stages.json")


def stage_vectors():
    from tests.custody_gap_stage import python_stages
    from tests.custody_gap_generator import SCENARIOS,snapshot
    from tests.custody_snapshot_generator import SCENARIOS as EVIDENCE, snapshot as evidence_snapshot,component_snapshot
    from kassiber.core.custody_compute import compute_custody_decisions
    books=[(case,snapshot(seed)) for seed,case in enumerate(SCENARIOS)]
    books += [("evidence_"+case,evidence_snapshot(seed)) for seed,case in enumerate(EVIDENCE)]
    books.append(("component_reservations",component_snapshot(0)))
    from tests.custody_channel_generator import CHANNEL_SCENARIOS,channel_snapshot
    books += [("channel_"+case,channel_snapshot(seed,case=case)) for seed,case in enumerate(CHANNEL_SCENARIOS)]
    result=[]
    for case,book in books:
        expected=python_stages(book)["gaps"]
        decisions=compute_custody_decisions(book)
        result.append(dict(case=case,input=book,expected=to_jsonable(expected,redact_config=False),
                           ignored=list(decisions.gap_ignored_transaction_ids),accounting_ignored=list(decisions.gap_accounting_ignored_transaction_ids)))
    for kind in ("digits","timestamp"):
        book=snapshot(0)
        book["loan_legs"]=[dict(transaction_id="source",role="loan_principal_out")]
        book["gap_worklist"]["rows"][0].update({"amount":"²"} if kind=="digits" else {"occurred_at":"0001-01-01T00:00:00+01:00"})
        result.append(dict(case="admission_"+kind,input=book,**outcome(lambda:python_stages(book))))
    return result


if __name__ == "__main__":
    text=json.dumps(vectors(),ensure_ascii=True,separators=(",",":"))+"\n"
    if "--check" in sys.argv:
        assert PATH.read_text()==text,PATH
    else:
        PATH.write_text(text)
    stage_text=json.dumps(stage_vectors(),ensure_ascii=True,separators=(",",":"))+"\n"
    if "--check" in sys.argv:assert STAGES_PATH.read_text()==stage_text,STAGES_PATH
    else:STAGES_PATH.write_text(stage_text)
