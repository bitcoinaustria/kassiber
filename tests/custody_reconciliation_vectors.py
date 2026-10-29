"""CPython reconciliation/compiler witnesses; derived values are test-only."""
from copy import deepcopy
from dataclasses import replace

def changed(value, **fields):
    value = deepcopy(value)
    for key, item in fields.items(): object.__setattr__(value, key, item)
    return value

import json
from pathlib import Path
import sys

from kassiber.core import custody_interpreters as interpreters
from kassiber.core.custody_compute import compute_components_stage, compute_evidence_stage, compute_custody_decisions
from kassiber.core.custody_component_claims import compile_component_quantity_claims
from kassiber.core.custody_native_reconciliation import reconcile_native_components
from kassiber.core.custody_quantity import ClaimPriority, QuantityClaim, QuantitySlice
from kassiber.transfers import onchain_transfer_scope
from kassiber.core.custody_allocations import allocate_msat_fifo
from kassiber.core.transfer_chronology import ChainOrderEvidence, chain_order_evidence, order_same_time_transfers
from types import SimpleNamespace
from tests.custody_reconciliation_generator import SCENARIOS, snapshot
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_reconciliation_cases.json"
HELPERS = PATH.with_name("custody_reconciliation_helpers.json")


def vectors():
    return [dict(case=case, input=(book := snapshot(i, case=case)), expected=to_jsonable(compute_custody_decisions(book).interpretation, redact_config=False)) for i, case in enumerate(SCENARIOS)]


def base(case="recovered"):
    book = snapshot(0, case=case)
    components, _ = compute_components_stage(book)
    canonical = compute_evidence_stage(book)["canonical_input"]
    observations = interpreters._observations_by_transaction(canonical)
    candidate = interpreters.compile_custody_interpreters(book["transactions"], canonical, wallet_refs_by_id={w["id"]: w for w in book["wallets"]}, profile=book["profile"], as_of=book["as_of"])
    claims = candidate.claims
    return dict(components=list(components), observations=observations, native_claims=list(claims), blocked_transaction_ids=set(), rows_by_transaction={r["id"]: r for r in book["transactions"]})


def operands():
    data=base();data['components'][0]['legs'][0]['transaction_id']='original-alias'
    yield 'journal_anchor_precedence','reviewed',dict(component=data['components'][0],observations=data['observations'])
    first=base();second=deepcopy(first)
    first['components'][0]['id']='z-first'
    second['components'][0]['id']='a-second'
    second['observations']={key+':second':replace(o,transaction_id=o.transaction_id+':second',anchor_transaction_id=o.anchor_transaction_id+':second',quantity_hash=o.quantity_hash+':second',wallet_id=o.wallet_id+':second') for key,o in second['observations'].items()}
    for leg in second['components'][0]['legs']:
        for key in ('transaction_id','anchor_transaction_id','wallet_id'):
            if leg.get(key): leg[key]+=':second'
    second['native_claims']=[changed(c,claim_id=c.claim_id+':second',source=replace(c.source,observation_hash=c.source.observation_hash+':second'),target=replace(c.target,observation_hash=c.target.observation_hash+':second')) for c in second['native_claims']]
    first['components'].extend(second['components']);first['observations'].update(second['observations']);first['native_claims'].extend(second['native_claims'])
    yield 'independent_component_order','reconcile',first
    for case,value in (('bad_component_value','bad'),('bad_component_type',[1])):
        data=base();data['components'][0]['allocations'][0]['source_amount_msat']=value
        yield case,'reconcile',data
    data=base('two_targets')
    data['observations']={key:replace(o,occurred_at='2026-01-01T00:00:00Z') for key,o in data['observations'].items()}
    yield 'same_time_fanout','reconcile',data
    data=base()
    observations=data['observations'];source=observations['route-out'];target=observations['route-in'];n=source.amount_msat;half=n//2
    spec=(('route-out','wallet-a','outbound',n,1),('branch-b-in','wallet-b','inbound',half,1),('branch-c-in','wallet-c','inbound',half,1),('branch-b-out','wallet-b','outbound',half,2),('branch-c-out','wallet-c','outbound',half,2),('merge-in','wallet-d','inbound',n,2),('merge-out','wallet-d','outbound',n,3),('route-in','wallet-e','inbound',n,3))
    data['observations']={id:replace(source if direction=='outbound' else target,transaction_id=id,anchor_transaction_id=id,quantity_hash=f'hash:{id}',wallet_id=wallet,direction=direction,amount_msat=amount,occurred_at=f'2026-01-0{day}T00:00:00Z',event_key=replace(source.event_key,native_event_id=f'{day:02x}'*32)) for id,wallet,direction,amount,day in spec}
    edges=(('route-out','branch-b-in',0,0,half),('route-out','branch-c-in',half,0,half),('branch-b-out','merge-in',0,0,half),('branch-c-out','merge-in',0,half,half),('merge-out','route-in',0,0,n))
    data['native_claims']=[QuantityClaim(str(i),QuantitySlice(f'hash:{a}',sa,sa+n),target=QuantitySlice(f'hash:{b}',ta,ta+n),state='internal_verified',priority=ClaimPriority.EXACT_NATIVE_EVENT,reason='exact_native') for i,(a,b,sa,ta,n) in enumerate(edges)]
    data['rows_by_transaction']={}
    yield 'branch_and_merge','reconcile',data
    for case,sources,sinks in (
        ('fifo_split',[['a',7],['b',5]],[['c',3],['d',8]]),
        ('fifo_zeros',[['z',0],['a',7]],[['empty',0],['c',0],['d',7]]),
        ('fifo_empty',[],[]),('fifo_zero_sources',[],[['z',0]]),
        ('fifo_insufficient',[['a',1]],[['b',2]]),
        ('fifo_huge',[['a',10**100],['b',10**100]],[['c',10**100+1]]),
    ):
        yield case,'fifo',dict(sources=sources,sinks=sinks)
    proof=lambda tx,parents=():dict(scope=['bitcoin','main',tx,'BTC'],parents=[['bitcoin','main',p,'BTC'] for p in parents])
    for case,transfers in (
        ('wallet_order',[dict(from_wallet='b',to_wallet='c',proof=None),dict(from_wallet='a',to_wallet='b',proof=None)]),
        ('wallet_cycle',[dict(from_wallet='b',to_wallet='a',proof=None),dict(from_wallet='a',to_wallet='b',proof=None)]),
        ('physical_order',[dict(from_wallet='b',to_wallet='a',proof=proof('child',['parent'])),dict(from_wallet='a',to_wallet='b',proof=proof('parent'))]),
        ('same_event',[dict(from_wallet='b',to_wallet='c',proof=proof('parent')),dict(from_wallet='a',to_wallet='b',proof=proof('parent'))]),
        ('hard_cycle',[dict(from_wallet='a',to_wallet='b',proof=proof('a',['b'])),dict(from_wallet='b',to_wallet='a',proof=proof('b',['a']))]),
        ('soft_cycle',[dict(from_wallet='a',to_wallet='b',proof=proof('a')),dict(from_wallet='b',to_wallet='c',proof=proof('b',['a'])),dict(from_wallet='c',to_wallet='a',proof=None)]),
        ('stable_ties',[dict(from_wallet='a',to_wallet='b',proof=None),dict(from_wallet='c',to_wallet='d',proof=None)]),
    ):
        yield case,'order',transfers
    yield 'source_fanout_dedup','source_order',[
        dict(source_hash='one',from_wallet='a',to_wallet='b',proof=None),
        dict(source_hash='one',from_wallet='a',to_wallet='c',proof=None),
        dict(source_hash='two',from_wallet='d',to_wallet='e',proof=None),
    ]
    row=base('same_time')['rows_by_transaction']['middle-out']
    yield 'closed','chronology',row
    for key,value in (('observation_graph_hash','stale'),('external_id','bad'),('observation_authority_version',None)):
        r=deepcopy(row);r[key]=value;yield key,'chronology',r
    from kassiber.core.chain_observer.provenance import canonical_graph_hash
    for i,vout in enumerate((None,True,0.0,'0',-1,0,10**100)):
        r=deepcopy(row);raw=json.loads(r['raw_json']);raw['vin'][0]['vout']=vout;r['raw_json']=json.dumps(raw);r['observation_graph_hash']=canonical_graph_hash(r['raw_json'])
        yield f'vout_{i}','chronology',r
    for case in SCENARIOS:
        yield case, "reconcile", base(case)
    for field, values in {
        "priority": (ClaimPriority.REVIEWED_COMPONENT, ClaimPriority.ACCOUNTING_CONVENTION),
        "state": ("custody_suspense",), "target": (None,),
    }.items():
        for i, value in enumerate(values):
            data = base(); data["native_claims"][0] = changed(data["native_claims"][0], **{field: value})
            yield f"claim_{field}_{i}", "reconcile", data
    for which in ("route-out", "middle-in", "middle-out", "route-in"):
        for field, value in (("authoritative_chain_observation", False), ("asset", "ETH"), ("wallet_id", "foreign"), ("fee_attribution", "unknown"), ("fee_msat", -1), ("amount_msat", 2_000_000)):
            data = base(); data["observations"][which] = replace(data["observations"][which], **{field: value})
            yield f"observation_{which}_{field}", "reconcile", data
        data = base(); data["blocked_transaction_ids"].add(which)
        yield f"blocked_{which}", "reconcile", data
    for field, value in (("chain", "lightning"), ("native_namespace", "payment_hash"), ("network", "other")):
        data = base()
        for which in ("route-out", "middle-in"):
            o = data["observations"][which]; data["observations"][which] = replace(o, event_key=replace(o.event_key, **{field:value}))
        yield f"event_{field}", "reconcile", data
    data=base(); data["observations"]["middle-in"] = replace(data["observations"]["middle-in"],wallet_id="wallet-a")
    yield "same_wallet", "reconcile", data
    for side in ("source", "target"):
        for kind in ("gap", "overlap", "short"):
            data=base(); c=data["native_claims"][0]; part=getattr(c,side)
            part=replace(part,start_msat=1) if kind=="gap" else replace(part,end_msat=part.end_msat-1)
            data["native_claims"][0]=changed(c,**{side:part})
            if kind=="overlap": data["native_claims"].append(data["native_claims"][0])
            yield f"slice_{side}_{kind}", "reconcile", data
    for field,value in (("effective_state","draft"),("conservation_mode","conversion"),("economic_terms",[{}])):
        data=base();data["components"][0][field]=value
        yield f"component_{field}", "reconcile",data
    data=base(); data['components'].append(deepcopy(data['components'][0]));data['components'][1]['id']='second'
    yield 'selected_anchors', 'reconcile', data
    for case in ("recovered","source_fee","residual_suspense","residual_fee","two_sources","two_targets"):
        data=base(case)
        component=data['components'][0]
        yield case,'reviewed',dict(component=component,observations=data['observations'])
        for key,value in (('id',''),('effective_state','draft')):
            c=deepcopy(component);c[key]=value
            yield f'{case}_{key}','reviewed',dict(component=c,observations=data['observations'])
    for case, mutate in (
        ('unavailable_source',lambda c,o:o.pop('route-out')),
        ('unavailable_target',lambda c,o:o.pop('route-in')),
        ('wrong_source_direction',lambda c,o:o.update({'route-out':replace(o['route-out'],direction='inbound')})),
        ('wrong_target_direction',lambda c,o:o.update({'route-in':replace(o['route-in'],direction='outbound')})),
        ('source_principal',lambda c,o:o.update({'route-out':replace(o['route-out'],amount_msat=1)})),
        ('target_principal',lambda c,o:o.update({'route-in':replace(o['route-in'],amount_msat=1)})),
        ('incomplete_principal',lambda c,o:o.update({'route-out':replace(o['route-out'],amount_msat=2_000_000)})),
        ('invalid_endpoints',lambda c,o:c['allocations'][0].update(source_leg_id='absent')),
        ('wrong_source_role',lambda c,o:c['legs'][0].update(role='destination')),
        ('unequal',lambda c,o:c['allocations'][0].update(sink_amount_msat=1)),
        ('zero',lambda c,o:c['allocations'][0].update(source_amount_msat=0,sink_amount_msat=0)),
        ('bad_amount',lambda c,o:c['allocations'][0].update(source_amount_msat='bad')),
        ('type_amount',lambda c,o:c['allocations'][0].update(source_amount_msat=[1])),
        ('inferred',lambda c,o:c.update(allocations=[])),
        ('zero_then_complete',lambda c,o:c['allocations'].append(dict(c['allocations'][0],id='zero',ordinal=-1,source_amount_msat=0,sink_amount_msat=0))),
        ('external',lambda c,o:c['legs'][1].update(role='external')),
        ('unsupported_role',lambda c,o:c['legs'][1].update(role='unsupported')),
        ('retained',lambda c,o:c['legs'][1].update(role='retained',transaction_id=None,anchor_transaction_id=None,notes='reviewed_residual:retained_custody')),
        ('stranded',lambda c,o:c['legs'][1].update(transaction_id=None,anchor_transaction_id=None)),
        ('missing_location',lambda c,o:c['legs'][1].update(transaction_id=None,anchor_transaction_id=None,wallet_id=None)),
    ):
        data=base();component=data['components'][0];observations=data['observations'];mutate(component,observations)
        yield case,'reviewed',dict(component=component,observations=observations)
    data=base('two_sources');data['components'][0]['allocations']=[]
    yield 'inferred_many_to_one','reviewed',dict(component=data['components'][0],observations=data['observations'])
    data=base();c=data['components'][0];o=data['observations'];target=c['legs'][1];target['amount_msat']//=2
    o['route-in']=replace(o['route-in'],amount_msat=target['amount_msat'])
    c['legs'].append(dict(target,id='extra-leg',transaction_id='extra-in',anchor_transaction_id='extra-in'))
    o['extra-in']=replace(o['route-in'],transaction_id='extra-in',anchor_transaction_id='extra-in',quantity_hash='extra-hash')
    c['allocations']=[]
    yield 'inferred_one_to_many','reviewed',dict(component=c,observations=o)
    for case in ('route','deferred_route','stranded_route','unfunded_route','unicode_location','split_route','multi_source_route','multi_source_split'):
        data=base('two_sources' if case.startswith('multi_source') else 'recovered');c=data['components'][0];o=data['observations']
        source=c['legs'][0];target=c['legs'][1];n=target['amount_msat']
        ghost=dict(target,id='ghost-in',transaction_id=None,anchor_transaction_id=None,wallet_id='untracked',rail='untracked')
        spend=dict(source,id='ghost-out',transaction_id=None,anchor_transaction_id=None,wallet_id='untracked',rail='untracked',amount_msat=n)
        c['legs'].extend([ghost,spend])
        outgoing=dict(c['allocations'][0],id='last',ordinal=10,source_leg_id=spend['id'],sink_leg_id=target['id'],source_amount_msat=n,sink_amount_msat=n)
        for a in c['allocations']: a['sink_leg_id']=ghost['id']
        c['allocations'].append(outgoing)
        if case=='deferred_route': outgoing['ordinal']=-1
        if case=='stranded_route': outgoing.update(source_amount_msat=n-1,sink_amount_msat=n-1)
        if case=='unfunded_route': spend['wallet_id']='elsewhere'
        if case=='unicode_location': ghost['asset']='ß';spend['asset']='SS'
        if case=='split_route':
            outgoing.update(source_amount_msat=n-1,sink_amount_msat=n-1)
            c['allocations'].append(dict(outgoing,id='last-split',ordinal=11,source_amount_msat=1,sink_amount_msat=1))
        if case=='multi_source_split':
            outgoing.update(source_amount_msat=n//4,sink_amount_msat=n//4)
            c['allocations'].append(dict(outgoing,id='last-split',ordinal=11,source_amount_msat=n-n//4,sink_amount_msat=n-n//4))
        yield case,'reviewed',dict(component=c,observations=o)


def helpers():
    result=[]
    for case,op,value in operands():
        v=dict(case=case,op=op,value=to_jsonable(value,redact_config=False))
        # Sets are tagged by the recorder; this local helper argument is a list.
        if op=='reconcile': v['value']['blocked_transaction_ids']=sorted(value['blocked_transaction_ids'])
        try:
            if op=='fifo':
                allocation=allocate_msat_fifo(**value)
                expected=dict(cells=[dict(source=c.source_id,sink=c.sink_id,amount=c.amount_msat) for c in allocation.cells],remaining=allocation.source_remaining)
            elif op in ('order','source_order'):
                transfers={str(i):SimpleNamespace(from_wallet_id=t['from_wallet'],to_wallet_id=t['to_wallet'],out_row={'custody_chain_order':ChainOrderEvidence(tuple(t['proof']['scope']),frozenset(tuple(p) for p in t['proof']['parents'])) if t['proof'] else None}) for i,t in enumerate(value)}
                expected=[int(key) for _,key in order_same_time_transfers([(i,('transfer',str(i))) for i in range(len(value))],transfers)]
                if op=='source_order': expected=list(dict.fromkeys(value[i]['source_hash'] for i in expected))
            elif op=='chronology':
                evidence=chain_order_evidence(value)
                expected=dict(scope=evidence.scope,parents=sorted(evidence.parents)) if evidence else None
            elif op=='reconcile':
                expected=reconcile_native_components(**value)
                if case=='independent_component_order': assert len(expected)==2
            else:
                compilation=compile_component_quantity_claims(value['component'],value['observations'])
                expected=[dict(claim_id=c.claim_id,source=dict(observation_hash=c.source.observation_hash,start_msat=c.source.start_msat,end_msat=c.source.end_msat),target=dict(observation_hash=c.target.observation_hash,start_msat=c.target.start_msat,end_msat=c.target.end_msat) if c.target else None,state=c.state,reason=c.reason,destination_kind='fee' if c.destination_kind=='fee' else None) for c in compilation.claims]
            v['expected']=to_jsonable(expected,redact_config=False)
        except Exception as exc: v['error']=getattr(exc,'code',type(exc).__name__)
        result.append(v)
    return result


if __name__=='__main__':
    for path, actual in ((PATH,vectors()),(HELPERS,helpers())):
        contents=json.dumps(actual,ensure_ascii=True,separators=(',',':'))+'\n'
        if '--check' in sys.argv: assert path.read_text()==contents,path
        else: path.write_text(contents)
