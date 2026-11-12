"""Raw private composition operands, compared against the Python compiler.

Issue and reviewed-conversion output is intentionally outside this checkpoint.
The independent connected/barrier oracle executes original reference AST nodes.
"""
import ast
from copy import deepcopy
import inspect
from pathlib import Path
from types import SimpleNamespace as NS
from unittest.mock import patch

from kassiber.core import custody_tax_projection as p
from kassiber.core.custody_quantity import QuantitySlice
from kassiber.core.custody_quantity_runtime import QuantityTaxEligibility, TaxExposurePool
from tests.custody_projection_money import observation
from tests.custody_snapshot import to_jsonable
from tests.test_custody_quantity import _observation, _row

PATH = Path(__file__).parent / 'fixtures/custody_projection_composition.json'
VALUATION_HOLDS = {'invalid_transaction_price', 'split_payout_requires_review'}



def source_rows(v):
    """The operand's rows in its source order: `row_order` when given, else
    by id, as the Rust compiler reads a JSON object (its keys sort)."""
    return [v['rows'][key] for key in (v.get('row_order') or sorted(v['rows']))]

def environment(v):
    obs=[observation(o) for o in v['observations']]
    decisions=[]
    for d in v['decisions']:
        d=deepcopy(d);d['source']=QuantitySlice(**d['source'])
        if d['target'] is not None:d['target']=QuantitySlice(**d['target'])
        decisions.append(NS(**d))
    eligibility=QuantityTaxEligibility(tuple(d for d in decisions if to_jsonable(d.source,redact_config=False) in [dict(q,**{'$type':'QuantitySlice'}) for q in v['eligible']]),(),None,pool_barriers=tuple((TaxExposurePool(**pool),tuple(order)) for pool,order in v['pool_barriers']))
    events=[NS(event_key=NS(**e['event_key']),legs=tuple(NS(**o) for o in e['legs'])) for e in v['events']]
    issues=[NS(reason='custody_component_incomplete',issue_type='component_claim_compile_failed',transaction_ids=(anchor,),issue_id=anchor,details={}) for anchor in v['blocked_anchor_ids']]
    state=NS(projection=NS(observations=obs,decisions=decisions),tax_eligibility=eligibility,canonical_input=NS(events=events),issues=issues,reviewed_conversion_pairs=())
    return obs,decisions,state


def fixed_eligibility(state, rows_by_id, blockers):
    """Private operands carry their tax eligibility as given.

    Valuation holds still block their transactions and add their quarantines;
    recomputing eligibility with them is a stage concern, compared through the
    custody shadow and journal process tests.
    """
    return state


def evaluate(v):
    with patch.object(p, 'with_tax_projection_blockers', fixed_eligibility):
        return _evaluate(v)


def _evaluate(v):
    try:
        obs,decisions,state=environment(v)
        root=ast.parse(inspect.getsource(p.compile_finalized_tax_projection)).body[0]
        env=dict(vars(p),state=state,decisions=decisions,observations={o.quantity_hash:o for o in obs},explicitly_blocked=set(v['blocked_transaction_ids']))
        if v['op']=='connected':
            start=next(i for i,n in enumerate(root.body) if isinstance(n,ast.AnnAssign) and n.target.id=='move_neighbors')
            end=next(i for i,n in enumerate(root.body) if isinstance(n,ast.AnnAssign) and n.target.id=='event_order_by_hash')
            exec(compile(ast.fix_missing_locations(ast.Module(body=root.body[start:end],type_ignores=[])),'<Python connected reference>','exec'),env)
            result=dict(blocked=sorted(env['effectively_blocked']))
        else:
            start=next(i for i,n in enumerate(root.body) if isinstance(n,ast.AnnAssign) and n.target.id=='event_order_by_hash')
            end=next(i for i,n in enumerate(root.body) if isinstance(n,ast.FunctionDef) and n.name=='is_blocked_by_basis_barrier')
            # Future annotations keep this exact nested reference independent
            # of the fixture's namespace types.
            module=ast.Module(body=[ast.ImportFrom(module='__future__',names=[ast.alias(name='annotations')],level=0),*root.body[start:end+1]],type_ignores=[])
            exec(compile(ast.fix_missing_locations(module),'<Python basis reference>','exec'),env)
            held=[o.quantity_hash for o in sorted(obs,key=lambda o:(o.occurred_at,o.quantity_hash)) if o.direction=='inbound' and o.transaction_id in v['rows'] and o.anchor_transaction_id not in v['non_event_transaction_ids'] and env['is_blocked_by_basis_barrier'](o)]
            if v['op']=='basis':result=[env['is_blocked_by_basis_barrier'](o) for o in obs]
            else:
                compiled=p.compile_finalized_tax_projection(v['profile'],source_rows(v),state,non_event_transaction_ids=v['non_event_transaction_ids'],blocked_transaction_ids=v['blocked_transaction_ids'],direct_payout_records=v['payouts'])
                result={key:getattr(compiled,key) for key in ('rows','intra_pairs','cross_asset_pairs','selected_move_ids')}
                result['barrier_held_receipts']=held
                # Valuation holds join the explicit blocks before the closure.
                held=[q['transaction_id'] for q in compiled.quarantines if q['reason'] in VALUATION_HOLDS]
                result['blocked_transaction_ids']=evaluate(dict(v,op='connected',blocked_transaction_ids=v['blocked_transaction_ids']+held))['expected']['blocked']
        return {'expected':to_jsonable(result,redact_config=False)}
    except Exception as exc:return {'error':type(exc).__name__}


def seed(op='compose'):
    obs=[to_jsonable(_observation(name,name,direction,100,fee_msat=7 if direction=='outbound' else 0),redact_config=False) for name,direction in [('source','outbound'),('target','inbound'),('other','inbound')]]
    rows={name:_row(name,name,direction,100,fee_msat=7 if direction=='outbound' else 0) for name,direction in [('source','outbound'),('target','inbound'),('other','inbound')]}
    def q(i,start=0,end=100):return dict(observation_hash=obs[i]['quantity_hash'],start_msat=start,end_msat=end)
    d=dict(source=q(0),target=q(1),state='internal_verified',reason='native',selected_claim_id='pair:durable:edge',atomic_bundle_id=None,component_id=None,transfer_policy=None,transfer_kind=None,destination_kind=None,external_economic_subtype=None)
    events=[dict(event_key={k:x for k,x in o['event_key'].items() if k!='$type'},legs=[o]) for o in obs]
    return dict(op=op,observations=obs,rows=rows,decisions=[d],eligible=[q(0)],events=events,pool_barriers=[],blocked_transaction_ids=[],blocked_anchor_ids=[],non_event_transaction_ids=[],payouts=[],profile=dict(id='p',workspace_id='w',tax_country='generic'))


def operands():
    def case(name,edit=lambda v:None,op='compose'):
        v=seed(op);edit(v);v['case']=name;return v
    yield case('selected_move')
    yield case('selected_reviewed_move',lambda v:v['decisions'][0].update(state='internal_reviewed'))
    yield case('anchor_alias',lambda v:(v['observations'][0].update(transaction_id='virtual-source'),v['observations'][1].update(transaction_id='virtual-target')))
    yield case('duplicate_slice_refusal',lambda v:v['decisions'].append(deepcopy(v['decisions'][0])))
    yield case('move_fallback',lambda v:v['decisions'][0].update(selected_claim_id=None))
    yield case('move_before_non_event',lambda v:v.update(non_event_transaction_ids=['source']))
    for marker in ['coinjoin',' PAYJOIN ','payment-in-coinjoin','unrecognized']:
        yield case('privacy_'+marker,lambda v,m=marker:v['rows']['target'].update(privacy_boundary=m))
    for ident in ['component:review:edge','pair:durable:edge','native:id']:
        yield case('cross_'+ident,lambda v,i=ident:(v['observations'][1].update(asset='LBTC'),v['decisions'][0].update(selected_claim_id=i)))
    yield case('cross_explicit',lambda v:(v['observations'][1].update(asset='ETH'),v['decisions'][0].update(component_id='explicit',transfer_kind='reviewed',transfer_policy='taxable')))
    for subtype in [None,'gift','lost','payment','']:
        yield case('external_'+str(subtype),lambda v,s=subtype:v['decisions'][0].update(target=None,state='external_confirmed',external_economic_subtype=s))
    yield case('external_presumed',lambda v:v['decisions'][0].update(target=None,state='external_presumed'))
    yield case('non_event_fee',lambda v:(v['decisions'][0].update(target=None,state='external_presumed'),v.update(non_event_transaction_ids=['source'])))
    yield case('reviewed_fee',lambda v:(v['decisions'][0].update(target=None,state='external_confirmed',destination_kind='fee',source=dict(v['decisions'][0]['source'],start_msat=1)),v['eligible'][0].update(start_msat=1)))
    yield case('unselected',lambda v:v.update(eligible=[]))
    yield case('unresolved_fee_only',lambda v:v['decisions'][0].update(target=None,state='custody_suspense'))
    yield case('rowless_receipt',lambda v:v['rows'].pop('other'))
    yield case('blocked_receipt',lambda v:v.update(blocked_anchor_ids=['other']))
    yield case('non_event_receipt',lambda v:v.update(non_event_transaction_ids=['other']))
    yield case('inbound_gap',lambda v:v['decisions'][0]['target'].update(start_msat=10,end_msat=80))
    def overlap(v):
        d=deepcopy(v['decisions'][0]);d['source'].update(start_msat=20,end_msat=40);d['target'].update(start_msat=5,end_msat=50);d.update(state='custody_suspense');v['decisions'].append(d)
        v['decisions'][0]['target'].update(start_msat=10,end_msat=80)
        v['eligible']=[]
    yield case('inbound_overlap_all_targets',overlap)
    def payout(v,**extra):
        v['decisions'][0].update(target=None,state='external_confirmed',reason='reviewed_direct_payout_source')
        v['payouts']=[dict(id='paid',out_transaction_id='source',payout_amount=50,payout_asset='LBTC',payout_fiat_value='2.50',policy='carrying-value',**extra)]
    yield case('payout_carry',payout)
    yield case('payout_split_hold',lambda v:(payout(v),v['decisions'][0]['source'].update(end_msat=50),v['eligible'][0].update(end_msat=50)))
    yield case('price_hold',lambda v:v['rows']['source'].update(fiat_rate='1e19'))
    yield case('payout_taxable',lambda v:(payout(v),v['payouts'][0].update(policy='taxable')))
    yield case('payout_missing_value',lambda v:(payout(v),v['payouts'][0].update(payout_fiat_value=None)))
    yield case('payout_bad_value',lambda v:(payout(v),v['payouts'][0].update(payout_fiat_value='bad')))
    yield case('source_bad_amount',lambda v:v['rows']['source'].update(amount=[1]))
    yield case('source_bad_numeric',lambda v:v['rows']['source'].update(amount='bad'))
    yield case('missing_source',lambda v:v['decisions'][0]['source'].update(observation_hash='missing'))
    for blocked in [['source'],['target'],['other'],['source','target']]:
        yield case('connected_'+','.join(blocked),lambda v,b=blocked:v.update(blocked_transaction_ids=b),op='connected')
    def path(v):
        d=deepcopy(v['decisions'][0]);d['source']['observation_hash']=v['observations'][1]['quantity_hash'];d['target']['observation_hash']=v['observations'][2]['quantity_hash'];v['decisions'].append(d);v['blocked_transaction_ids']=['source']
    yield case('connected_transitive',path,op='connected')
    yield case('connected_external_break',lambda v:(path(v),v['decisions'][1].update(state='external_confirmed')),op='connected')
    yield case('connected_composition',path)
    def barriers(v):
        o=observation(v['observations'][0]);pool=to_jsonable(TaxExposurePool.from_observation(o),redact_config=False);pool.pop('$type')
        e=v['events'][0];order=[o.occurred_at,*[e['event_key'][k] for k in ('chain','network','native_namespace','native_event_id')]]
        v['pool_barriers']=[[pool,order]]
    for name,edit in [
        ('equal_distinct',lambda v:None),('earlier',lambda v:v['events'][1]['legs'][0].update(occurred_at='2025')),
        ('later',lambda v:v['events'][1]['legs'][0].update(occurred_at='2027')),
        ('missing_order',lambda v:v.update(events=[])),
        ('profile_isolation',lambda v:v['observations'][1].update(profile_id='another')),
        ('network_isolation',lambda v:v['observations'][1]['event_key'].update(network='regtest')),
        ('asset_isolation',lambda v:v['observations'][1].update(asset='ETH')),
        ('liquid_shared_pool',lambda v:(v['observations'][1].update(asset='LBTC'),v['observations'][1]['event_key'].update(network='liquidv1'))),
        ('same_event',lambda v:v['events'][0]['legs'].append(v['events'].pop(1)['legs'][0])),
    ]:
        yield case('basis_'+name,lambda v,e=edit:(barriers(v),e(v)),op='basis')
    yield case('basis_composition',barriers)
    def minimum_event(v):
        barriers(v)
        v['events'][0]['legs'].append(v['events'].pop(1)['legs'][0])
        v['events'][0]['legs'][1]['occurred_at']='2025'
        v['pool_barriers'][0][1][0]='2025'
    yield case('basis_event_minimum',minimum_event,op='basis')
    def split(v):
        v['decisions'][0]['source']['end_msat']=50
        v['decisions'][0]['target']['end_msat']=50
        second=deepcopy(v['decisions'][0])
        second['source'].update(start_msat=50,end_msat=100)
        second['target'].update(start_msat=50,end_msat=100)
        second['selected_claim_id']='second'
        v['decisions'].insert(0,second)
        v['eligible']=[deepcopy(d['source']) for d in v['decisions']]
    yield case('split_fee_once_and_order',split)
    def bundle(v):
        split(v)
        for d in v['decisions']:d.update(atomic_bundle_id='bundle',component_id='review',reason='reviewed_custody_component')
        fee=deepcopy(v['decisions'][0]);fee.update(target=None,state='external_confirmed',destination_kind='fee')
        fee['source'].update(start_msat=100,end_msat=107)
        v['decisions'].append(fee);v['eligible'].append(deepcopy(fee['source']))
    yield case('bundle_consumed_reviewed_fee',bundle)
    yield case('bundle_additive_refund',lambda v:(bundle(v),[d.update(transfer_kind='swap-refund') for d in v['decisions']]))
    yield case('target_missing',lambda v:v['decisions'][0]['target'].update(observation_hash='missing'))
    yield case('payout_admission_before_sort',lambda v:(v['decisions'][0]['source'].update(observation_hash='missing'),v['rows']['source'].update(amount=[1]),v.update(payouts=[dict(out_transaction_id='source')])))
    yield case('at_nonbitcoin_carry',lambda v:(payout(v),v['profile'].update(tax_country='AT'),v['payouts'][0].update(payout_asset='ETH')))
    yield case('generic_nonbitcoin_taxable',lambda v:(payout(v),v['payouts'][0].update(payout_asset='ETH')))
    yield case('payout_alias_not_rail',lambda v:(payout(v),v['payouts'][0].update(payout_asset='L-BTC')))
    yield case('fee_blocked_at_later_event',lambda v:(barriers(v),v['events'][0]['legs'][0].update(occurred_at='2027'),v.update(eligible=[])))



def scenario(v,variant):
    v=deepcopy(v)
    factor=(1,3,10**60+1,10**120+7)[variant%4] + (variant//4)*10**160
    for o in v['observations']:
        for k in ['amount_msat','principal_msat','fee_msat','wallet_delta_msat','wallet_movement_msat']:
            if type(o.get(k)) is int:o[k]*=factor
    for row in v['rows'].values():
        for k in ('amount','fee'):
            if type(row.get(k)) is int:row[k]*=factor
    for d in v['decisions']:
        for q in (d['source'],d['target']):
            if q:
                q['start_msat']*=factor;q['end_msat']*=factor
    for q in v['eligible']:q['start_msat']*=factor;q['end_msat']*=factor
    for p in v['payouts']:
        if type(p.get('payout_amount')) is int:p['payout_amount']*=factor
    return v
