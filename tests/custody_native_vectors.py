"""Frozen Python branch witnesses; helper operands never enter snapshot v1."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core import custody_native_transitions as n, transfer_matching as m
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from kassiber.core.htlc_parser import parse_htlc_redeem_script,extract_from_claim_witness,extract_from_refund_witness
from tests.custody_native_generator import native_snapshot,NATIVE_SCENARIOS
from tests.custody_native_partial import candidate_projection
from tests.custody_snapshot import to_jsonable

PATH=Path(__file__).parent/'fixtures/custody_native_helpers.json'

def operands():
    for i,case in enumerate(NATIVE_SCENARIOS):
        book=native_snapshot(i,case=case)
        yield case,'matcher',book['transactions']
        yield case,'presence',book['transactions']
    for source_case in ('lnd_settle','timeout_refund','raw_claim','raw_refund'):
        book=native_snapshot(0,case=source_case)
        row=next(r for r in book['transactions'] if r['direction']=='inbound')
        op='refund' if 'refund' in source_case else 'claim'
        yield source_case,op,row
        for field,values in {
            'observation_authority_version':(None,False,2),
            'observation_graph_hash':('stale',None),
            'observation_observer_kinds_json':('bad','{}','[]','["bitcoinrpc"]',None),
        }.items():
            for i,v in enumerate(values):
                r=deepcopy(row);r[field]=v;yield f'{source_case}_{field}_{i}',op,r
        for path,values in {
            ('observer',):(None,[],{},'esplora','bitcoinrpc'),
            ('htlc_spend',):(None,False,[],{}),
            ('htlc_spend','version'):(None,True,0,1.0,2,'1'),
            ('htlc_spend','role'):(None,[],{},'other'),
            ('htlc_spend','input_count'):(None,True,0,2,1.0),
            ('htlc_spend','funding_txid'):(None,'invalid','55'*32,'44'*32),
            ('htlc_spend','funding_vout'):(None,True,-1,1,0.0,'0'),
            ('htlc_spend','payment_hash'):(None,'00'*32,'BAD'),
            ('vin',):(None,{},[],[None],[{}],[{},{}]),
        }.items():
            for i,v in enumerate(values):
                r=deepcopy(row);raw=json.loads(r['raw_json']);target=raw
                for key in path[:-1]:
                    if not isinstance(target.get(key),dict):target[key]={}
                    target=target[key]
                target[path[-1]]=v;r['raw_json']=json.dumps(raw);r['observation_graph_hash']=canonical_graph_hash(r['raw_json']);r['observation_quantity_hash']=canonical_observed_quantity_hash(r)
                yield f'{source_case}_raw_{"_".join(path)}_{i}',op,r
    # Witness parsing byte/length/template alternatives, including malformed stacks.
    row=next(r for r in native_snapshot(0,case='raw_claim')['transactions'] if r['direction']=='inbound')
    witness=json.loads(row['raw_json'])['vin'][0]['witness'];script=bytes.fromhex(witness[-1])
    yield 'valid','script',script.hex()
    yield 'reverse','script','82012088'+script.hex()
    for push in (b'\x00',b'\x51',b'\x60',b'\x05\x01\x02\x03\x04\x05'):
        yield 'locktime_push_'+push.hex(),'script',(script[:59]+push+script[63:]).hex()
    for i in range(len(script)):
        yield f'truncated_{i}','script',script[:i].hex()
    for at in (0,1,22,23,24,58,59,63,64,65,99,100):
        for value in (0,1,5,6,0x50,0x51,0x60,0x61,0xff):
            changed=bytearray(script);changed[at]=value;yield f'byte_{at}_{value}','script',changed.hex()
    yield 'trailing','script',script.hex()+'00'
    yield 'empty','script',''
    for i,v in enumerate((None,{},[],[''],witness, [witness[0],'',witness[-1]], [witness[0],'00'*32,witness[-1]], [witness[0],witness[1].upper(),witness[-1]], [witness[0], ' '.join(witness[1][j:j+2] for j in range(0,64,2)),witness[-1]], [witness[0],witness[1][:1]+' '+witness[1][1:],witness[-1]], [1,witness[-1]],[None,witness[-1]],[witness[0],'gg',witness[-1]],[witness[0],'00',witness[-1]])):
        yield f'stack_{i}','witness',v
    # Claim outpoint/witness admission independently of closed attestation.
    for key,values in {'txid':(None,False,'x','44'*32),'vout':(None,True,-1,'0',0.0,0,float('inf')),'witness':(None,{},[],[None],['zz'])}.items():
        for i,v in enumerate(values):
            r=deepcopy(row);raw=json.loads(r['raw_json']);raw['vin'][0][key]=v;r['raw_json']=json.dumps(raw);yield f'claim_vin_{key}_{i}','claim',r
    for v in (None,'not-json','[]','{"tx":null}','{"vin":{}}'):
        r=deepcopy(row);r['raw_json']=v;yield 'raw_'+str(v),'claim',r
    # Refund metadata precedence, including malformed fallback and alias wrappers.
    refund=next(r for r in native_snapshot(0,case='raw_refund')['transactions'] if r['direction']=='inbound')
    for key,values in {'swap_refund_funding_outpoint':('44'*32+':0','44'*32+':bad','44'*32,'44'*32+':-1',':0','44'*32+':٠','44'*32+':True'), 'swap_refund_funding_txid':(None,'','44'*32,('44'*32).upper(),'other'), 'swap_refund_funding_vout':(None,'',0,1,False,'0','bad',[],{},1.2)}.items():
        for i,v in enumerate(values):
            r=deepcopy(refund);r[key]=v
            if key=='swap_refund_funding_vout':r['swap_refund_funding_txid']='44'*32
            yield f'refund_{key}_{i}','refund',r
    raw=json.loads(refund['raw_json'])
    for key,value in [('tx',raw),('vin',raw['vin']+raw['vin']),('vin',raw['vin']+[dict(raw['vin'][0],txid='55'*32)]),('vin',[dict(raw['vin'][0],txinwitness=raw['vin'][0]['witness'],witness=None)])]:
        r=deepcopy(refund);r['raw_json']=json.dumps({key:value} if key=='tx' else dict(raw,**{key:value}));yield f'refund_wrapper_{key}_{len(str(value))}','refund',r
    # Funding output conversion/index precedence (unbounded and malformed indices).
    source=next(r for r in native_snapshot(0,case='timeout_refund')['transactions'] if r['direction']=='outbound')
    for index in (None,False,0,'0','٠',-1,'bad',[],10**100,float('inf')):
        for value in (None,False,0,-1,100000,'100000',1.25,'0.00100000',{},float('inf')):
            for key in ('value','value_sats'):
                r=deepcopy(source);r['raw_json']=json.dumps({'vout':[{'n':index,key:value}]});yield f'output_{index}_{value}_{key}','output',dict(row=r,index=0)
    # Exact and competing provider paths, legacy importer and unknown-network populations.
    for case in ('raw_claim','raw_refund','source_untrusted','provider_conflict','provider_same_edge','provider_only','duplicate_ln','duplicate_chain'):
        rows=native_snapshot(0,case=case)['transactions']
        for field,values in {'amount':(0,-1,'bad',[],{},False,10**400), 'fee':('bad',[],{},-1,0), 'excluded':(True,), 'direction':('other',), 'payment_hash':('bad','  '+'A'*64+' ',False)}.items():
            for i,v in enumerate(values):
                r=deepcopy(rows);r[0][field]=v;yield f'matcher_{case}_{field}_{i}','matcher',r
    for fee in (None, float('inf'), float('-inf')):
        r=native_snapshot(0,case='raw_claim')['transactions']
        next(row for row in r if row['direction']=='outbound')['fee']=fee
        yield 'outbound_fee_'+str(fee),'matcher',r
    # Provider candidates with known chain scopes exercise exactness, alias and contradiction admission.
    rows=native_snapshot(0,case='timeout_refund')['transactions']
    rows.sort(key=lambda r:r['direction']!='outbound')
    for row in rows:
        raw=row['raw_json'];raw=json.loads(raw) if isinstance(raw,str) else raw
        raw.update(provider='boltz',swap_id='synthetic-provider',flow='refund',send_txid=rows[0]['external_id'],receive_txid=rows[1]['external_id'],send_amount_msat=rows[0]['amount'],receive_amount_msat=rows[1]['amount']);row['raw_json']=json.dumps(raw)
    target_raw=json.loads(rows[1]['raw_json']);target_raw.pop('htlc_spend',None);rows[1]['raw_json']=json.dumps(target_raw)
    for key in ('swap_refund_funding_txid','swap_refund_funding_vout','swap_refund_funding_outpoint'):rows[1].pop(key,None)
    yield 'provider_exact','matcher',rows
    r=deepcopy(rows);raw=json.loads(r[1]['raw_json']);raw['send_amount_msat']+=1;r[1]['raw_json']=json.dumps(raw)
    yield 'provider_independent_amount_conflict','matcher',r
    for side in (0,1):
        r=deepcopy(rows);r.append(dict(r[side],id='provider-duplicate'))
        yield 'provider_duplicate_'+str(side),'matcher',r
    for field,values in {'provider':(None,'boltz\ud800','bull','boltz-regtest','other',''), 'swap_id':(None,'','other','\ud800','\U000f0000'), 'flow':(None,'', 'chain','reverse','submarine','unknown'), 'status':('refunded','pending'), 'send_txid':(None,'bad','55'*32), 'receive_txid':(None,'bad','55'*32), 'send_amount_msat':(None,True,-1,'100000000','١٠٠٠٠٠٠٠٠','²','፩','𝟙','bad',1.0,{}), 'receive_amount_msat':(None,-1,'99500000','bad'), 'sendTxid':('55'*32,), 'source':('other',), 'swapId':('different',), 'swap_type':('chain',), 'state':('other',)}.items():
        for i,v in enumerate(values):
            r=deepcopy(rows);raw=json.loads(r[0]['raw_json']);raw[field]=v;r[0]['raw_json']=json.dumps(raw);yield f'provider_{field}_{i}','matcher',r


    for field in ('provider','swap_id','flow','send_txid','receive_txid'):
        r=deepcopy(rows)
        for row in r:
            raw=json.loads(row['raw_json'])
            if field=='provider':raw[field]='bull'
            elif field=='swap_id':raw['id']=raw.pop(field)
            else:raw.pop(field,None)
            row['raw_json']=json.dumps(raw)
        yield 'provider_both_'+field,'matcher',r
    r=deepcopy(rows);r[1]['amount']=r[0]['amount']+1
    yield 'provider_negative_fee','matcher',r
    for side in (0,1):
        r=deepcopy(rows);r[side]['external_id']=None;raw=json.loads(r[side]['raw_json']);raw['send_txid' if side==0 else 'receive_txid']='55'*32;r[side]['raw_json']=json.dumps(raw)
        yield 'provider_graph_route_'+str(side),'matcher',r
    for source_asset,target_asset in (('BTC','LBTC'),('LBTC','BTC')):
        r=native_snapshot(0,case='source_untrusted')['transactions']
        for row in r:row.update(wallet_kind='descriptor',payment_hash_source='importer')
        r[0]['asset']=source_asset;r[1]['asset']=target_asset
        yield 'peg_'+source_asset,'matcher',r
    for amount,shortfall in ((1_000_000_000,10_000_000),(1_000_000_000,15_000_000),(100_000_000,2_500_000)):
        r=native_snapshot(0,case='raw_claim')['transactions'];r.sort(key=lambda r:r['direction']!='outbound');r[0]['amount']=amount;r[1]['amount']=amount-shortfall
        yield f'fee_threshold_{amount}_{shortfall}','matcher',r
    for payload in ({'htlc_spend':True},{'vin':[{'witness':['00']}]},{'tx':{'vin':[{'txinwitness':['00']}]}}):
        for raw in (payload,json.dumps(payload)):
            yield 'presence_raw_'+str(raw),'presence',[dict(raw_json=raw)]
    yield 'presence_escaped_gate','presence',[dict(raw_json='{"vin":[{"w'+chr(92)+'u0069tness":["00"]}]}')]
    # Independent candidate-edge deduplication/cluster witnesses. Candidates
    # retain Python dataclass construction; no helper operand enters snapshot v1.
    c=m.suggest_swap_candidates(native_snapshot(0)['transactions'],include_heuristics=False,now_iso='2026-01-01T00:00:00Z')[0]
    from dataclasses import asdict
    yield 'finalize_signed_fee_sort','finalize',[
        asdict(c),
        dict(asdict(c),out_id='negative-large',in_id='large-target',swap_fee_msat=-10_000_000),
        dict(asdict(c),out_id='negative-small',in_id='small-target',swap_fee_msat=-100),
    ]
    for changes in ((dict(method='htlc_refund'),), (dict(in_id='other'),), (dict(out_id='other',in_id='another'),), (dict(confidence='strong'),), (dict(method='provider_swap_id',confidence='exact'),), (dict(out_id='other'),dict(out_id='other',in_id='another'),dict(out_id='last',in_id='another'))):
        yield 'finalize_'+str(changes),'finalize',[asdict(c),*(dict(asdict(c),**fields) for fields in changes)]

    r=native_snapshot(0,case='source_untrusted')['transactions']
    for row in r:row.update(raw_json='{}',config_json='{}',wallet_config_json='{}',payment_hash_source='importer')
    yield 'both_unknown_domains','matcher',r
    for case in ('timeout_refund','policy_taxable','policy_austria','alias_asset'):
        yield 'production_'+case,'production',dict(snapshot=native_snapshot(0,case=case),occupied=[],overrides={})
    b=native_snapshot(0);b['profile']['tax_country']='unsupported'
    yield 'policy_refusal','production',dict(snapshot=b,occupied=[],overrides={})
    # Compiler-only operands model canonical aliases and reservations without
    # adding derived data to the raw snapshot contract.
    for occupied in ([],['send'],['claim'],['send','claim'],['other']):
        yield 'occupied_'+str(occupied),'production',dict(snapshot=native_snapshot(0),occupied=occupied,overrides={})
    for field,values in {'amount_msat':(0,-1,100_000_001), 'asset':('ETH',), 'authoritative_chain_observation':(False,), 'occurred_at':(None,'unknown','2025-10-10','0001-01-01T00:00:00+01:00')}.items():
        for id in ('send','claim'):
            for i,v in enumerate(values):yield f'canonical_{id}_{field}_{i}','production',dict(snapshot=native_snapshot(0),occupied=[],overrides={id:{field:v}})
    for id in ('send','claim'):
        yield 'missing_'+id,'production',dict(snapshot=native_snapshot(0),occupied=[],overrides={id:None})
    for expires in (None,'bad','2026-01-01T00:00:00Z','0001-01-01T00:00:00+01:00','9999-12-31T23:59:59.999999Z'):
        for now in ('bad','2026-01-01T00:00:00Z'):
            b=native_snapshot(0);b['as_of']=now;b['pair_dismissals']=[dict(out_transaction_id='send',in_transaction_id='claim',expires_at=expires)]
            yield f'dismissal_{expires}_{now}','production',dict(snapshot=b,occupied=[],overrides={})
    for case in ('lnd_settle','fee_next_day','duplicate_ln','duplicate_occupied','provider_conflict','privacy_native'):
        b=native_snapshot(0,case=case)
        for occupied in (['send'],['claim'],['duplicate']):yield case+str(occupied),'production',dict(snapshot=b,occupied=occupied,overrides={})


def evaluate(op,value):
    if op=='production':
        from dataclasses import replace
        from kassiber.core.custody_compute import compute_evidence_stage
        from kassiber.core.custody_interpreters import _observations_by_transaction
        b=value['snapshot'];observations=_observations_by_transaction(compute_evidence_stage(b)['canonical_input'])
        for id,fields in value['overrides'].items():
            if fields is None:observations.pop(id,None)
            else:observations[id]=replace(observations[id],**fields)
        pairs,proofs,quarantines=n.compile_native_transitions(b['transactions'],observations,profile=b['profile'],occupied_ids=set(value['occupied']),dismissals=b['pair_dismissals'],as_of=b['as_of'])
        return dict(pairs=pairs,proofs=[dict(key=key,proof=dict(candidate=candidate_projection(p.candidate),source_quantity_hash=p.source_quantity_hash,target_quantity_hash=p.target_quantity_hash,policy=p.policy)) for key,p in proofs.items()],quarantines=quarantines,candidates=[candidate_projection(c) for c in m.suggest_swap_candidates(b['transactions'],include_heuristics=False,now_iso=b['as_of'])])
    if op=='finalize':
        candidates=m.finalize_candidate_conflicts([m.SwapCandidate(**v) for v in value])
        candidates.sort(key=lambda c:(0 if c.confidence=='exact' else 1,abs(c.swap_fee_msat),c.out_occurred_at,c.out_id,c.in_id))
        return [candidate_projection(c) for c in candidates]
    if op=='matcher':return [candidate_projection(c) for c in m.suggest_swap_candidates(value,include_heuristics=False,now_iso='2026-01-01T00:00:00Z')]
    if op=='presence':return n.has_native_transition_evidence(value)
    if op=='claim':return m._row_has_verified_unique_claim_outpoint(value)
    if op=='refund':return m._refund_funding_reference(value)
    if op=='output':return m._funding_output_amount_msat(value['row'],value['index'])
    if op=='script':
        r=parse_htlc_redeem_script(bytes.fromhex(value));return r.hashlock160 if r else None
    if op=='witness':
        if not isinstance(value,list) or any(not isinstance(v,str) for v in value):return None
        try:w=[bytes.fromhex(v) for v in value]
        except ValueError:return None
        c=extract_from_claim_witness(w)
        if c:return True,c.payment_hash
        return (False,None) if extract_from_refund_witness(w) else None
    raise AssertionError(op)


def vectors():
    result=[]
    for case,op,value in operands():
        item=dict(case=case,op=op,value=value)
        try:item['expected']=evaluate(op,value)
        except Exception as exc:item['error']=type(exc).__name__
        result.append(to_jsonable(item,redact_config=False))
    return result

if __name__=='__main__':
    text=json.dumps(vectors(),indent=2,sort_keys=True,ensure_ascii=True,allow_nan=False)+'\n'
    if '--write' in sys.argv:PATH.write_text(text)
    else:assert PATH.read_text()==text
