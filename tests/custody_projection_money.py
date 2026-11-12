"""Independent operands for the second partial projection checkpoint.

The fee oracle executes the original nested Python helpers and arithmetic AST,
without selecting rows, opening books, or copying their implementation here.
"""
import ast
from copy import deepcopy
from functools import lru_cache
import inspect
from types import SimpleNamespace

from kassiber.core import custody_tax_projection as p
from kassiber.core.custody_quantity import QuantitySlice
from tests.custody_snapshot import to_jsonable
from tests.test_custody_quantity import _observation, _row

OPS = {'slice', 'payout_valid', 'payout_time', 'payout_settlement', 'payout_price', 'fees'}


def observation(v):
    from kassiber.core.custody_evidence import CanonicalEventKey
    from kassiber.core.custody_quantity import QuantityObservation
    v=deepcopy(v);v.pop('$type',None)
    event=v['event_key'];event.pop('$type',None)
    v['event_key']=CanonicalEventKey(**event)
    return QuantityObservation(**v)


@lru_cache
def fee_code():
    root=ast.parse(inspect.getsource(p.compile_finalized_tax_projection)).body[0]
    helpers=[node for node in root.body if isinstance(node,ast.FunctionDef) and node.name in ('pair_bundle','fee_for')]
    scan=next(node for node in root.body if isinstance(node,ast.For) and any(isinstance(child,ast.Name) and child.id=='fee_by_pair_bundle' for child in ast.walk(node)))
    move=next(node for node in ast.walk(root) if isinstance(node,ast.If) and any(isinstance(child,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='projected_fee' for t in child.targets) for child in node.body))
    arithmetic=[]
    for node in move.body:
        if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='out_row' for t in node.targets):break
        arithmetic.append(node)
    return tuple(compile(ast.fix_missing_locations(ast.Module(body=[ast.ImportFrom(module='__future__',names=[ast.alias(name='annotations')],level=0),*nodes],type_ignores=[])), '<Python projection fee reference>', 'exec') for nodes in (helpers+[scan],arithmetic))


def fees(v):
    decisions=[]
    for item in v['decisions']:
        d=dict(atomic_bundle_id=None,selected_claim_id=None,component_id=None,reason=None,transfer_kind=None,destination_kind=None)
        d.update(item);d['source']=QuantitySlice(**d['source']);decisions.append(SimpleNamespace(**d))
    env=dict(target=object(),decisions=decisions,observations={k:SimpleNamespace(**o) for k,o in v['observations'].items()}, allocated_fee_sources=set(),fee_by_pair_bundle={},consumed_fee_bundles=set())
    setup,move=fee_code();exec(setup,env)
    result=[]
    for action in v['actions']:
        decision=decisions[action['decision']];env['decision']=decision
        if action['kind']=='raw':result.append(env['fee_for'](decision))
        elif action['kind']=='move':exec(move,env);result.append(env['projected_fee'])
        else:
            bundle=env['pair_bundle'](decision)
            result.append(None if bundle is not None and bundle in env['consumed_fee_bundles'] else decision.source.amount_msat)
    return result


def evaluate(v):
    op=v['op']
    if op=='fees':return fees(v)
    if op=='slice':return p._projected_slice_row(observation(v['observation']),QuantitySlice(**v['quantity']),v['rows'],side=v['side'],fee_msat=v['fee'])
    if op=='payout_valid':return p._valid_direct_payouts_by_source(v['records'],v['rows'])
    if op=='payout_time':return p._direct_payout_occurred_at(v['payout'])
    if op=='payout_settlement':return p._direct_payout_settlement_rows(observation(v['observation']),v['payout'],v['rows'])
    if op=='payout_price':
        # Execute the unmodified reference price override branch with only its
        # already-selected operands. Selection remains outside this checkpoint.
        root=ast.parse(inspect.getsource(p.compile_finalized_tax_projection))
        branch=next(node for node in ast.walk(root) if isinstance(node,ast.If) and any(isinstance(child,ast.Name) and child.id=='payout_value' for child in ast.walk(node.test)))
        env=dict(vars(p),projected=deepcopy(v['row']),payout_value=v['payout'].get('payout_fiat_value'),decision=SimpleNamespace(source=SimpleNamespace(amount_msat=v['amount'])),source=SimpleNamespace(anchor_transaction_id='source'))
        exec(compile(ast.fix_missing_locations(ast.Module(body=[branch],type_ignores=[])),'<Python payout price reference>','exec'),env)
        return env['projected']
    raise AssertionError(op)


def operands():
    obs=to_jsonable(_observation('source','wallet','outbound',100,fee_msat=7),redact_config=False)
    row=_row('source','wallet','outbound',100,fee_msat=7)
    def slice_case(case,changes=None,**extra):
        return dict(case='slice_'+case,op='slice',observation=deepcopy(obs),rows={'source':dict(row,**(changes or {}))},quantity=dict(observation_hash='quantity:other',start_msat=1,end_msat=99),fee=7,side='external',**extra)
    for case,changes in [
        ('exact',dict(fiat_value_exact='1.23456789012345678901234567890123456')),
        ('float',dict(fiat_value=0.1)),('prefer_exact',dict(fiat_value_exact='1.00',fiat_value=9)),
        ('raw_gross',dict(amount=107,amount_includes_fee=True,fiat_value_exact='3')),
        ('rounding_denominator',dict(amount=int('12345678901234567890123456789012345678901234567890123456789012345678901'),fiat_value_exact='123456789.50000000000000000000000005')),
        # Keep an admitted rate between the finite-price floor and the 1e19
        # limit: this 33-digit amount's rounded 32-digit BTC denominator changes
        # the last digit of the rate.
        ('rounding_denominator_priced',dict(amount=int('123456789012345678901234567890123'),fiat_value_exact='423456789.50000000000000000000000005e10')),
        ('one_msat',dict(amount=1,fiat_value_exact='0.0000000000000000000000000000000005')),
        ('half_even_down',dict(amount=10**11,fiat_value_exact='1.'+'0'*30+'25')),
        ('half_even_up',dict(amount=10**11,fiat_value_exact='1.'+'0'*30+'35')),
        ('unit_rate',dict(fiat_rate=0,fiat_rate_exact='kept',fiat_value_exact='junk')),
        ('empty_exact',dict(fiat_value_exact='',fiat_value='3.50')),('invalid_exact',dict(fiat_value_exact='junk',fiat_value=3)),
        ('negative',dict(fiat_value_exact='-1.00')),('signed_zero',dict(fiat_value_exact='-0.00')),
        ('signed_zero_wide',dict(amount=10**60+1,fiat_value_exact='-0.00')),
        ('nan',dict(fiat_value_exact='NaN0012')),('snan',dict(fiat_value_exact='sNaN')),('inf',dict(fiat_value_exact='Infinity')),('negative_inf',dict(fiat_value_exact='-Infinity')),('negative_nan',dict(fiat_value_exact='-NaN0012')),
        ('derived_overflow',dict(amount=1,fiat_value_exact='1e308')),
        ('price_before_amount',dict(amount='bad',fiat_value_exact='Infinity')),
        ('price_before_graph',dict(observation_authority_version=1,amount=[1],fiat_rate='Infinity')),
        ('overflow',dict(fiat_value_exact='1e999999')),('underflow',dict(fiat_value_exact='1e-1000040')),
        ('unicode_decimal',dict(fiat_value_exact=' ١_٢.٣٤ ')),('false_amount',dict(amount=0,fiat_value_exact='3')),
        ('negative_amount',dict(amount=-1,fiat_value_exact='3')),('bad_amount',dict(amount='bad')),('amount_container',dict(amount=[1])),
        ('rowless',{}),('empty_value',dict(fiat_value_exact='',fiat_value='')),
    ]:
        v=slice_case(case,changes)
        if case=='rowless':v['rows']={}
        yield v
    base=dict(id='payout',out_transaction_id='source',out_amount=100,payout_amount=3,payout_fiat_value='1.00',payout_asset='lbtc',payout_occurred_at=None)
    for case,changes in [('whole',{}),('partial',dict(out_amount=1)),('too_large',dict(out_amount=101)),('zero',dict(out_amount=0)),('negative',dict(out_amount=-1)),('default',dict(out_amount='')),('none',dict(out_amount=None)),('unicode',dict(out_amount='９９')),('float',dict(out_amount=99.9)),('bad',dict(out_amount='bad')),('type',dict(out_amount=[1])),('missing',dict(out_transaction_id='absent'))]:
        yield dict(case='payout_valid_'+case,op='payout_valid',records=[dict(base,**changes)],rows={'source':row})
    for case,source in [('inclusive',dict(row,amount=107,amount_includes_fee=True)),('inbound',dict(row,direction='inbound',amount_includes_fee=True)),('bad_fee',dict(row,fee='bad'))]:
        yield dict(case='payout_valid_'+case,op='payout_valid',records=[dict(base,out_amount=101)],rows={'source':source})
    yield dict(case='payout_valid_last_valid',op='payout_valid',records=[base,dict(base,id='second',out_amount=99),dict(base,id='invalid',out_amount=101)],rows={'source':row})
    times=[None,'','2025-01-02','20250301T090030Z',' 2025-03-01t09:00:30z ', '2025-03-01 09:00 UTC','2025-03-01T09 GMT','2025-03-01T09:00:30,123456789 +0130','2025-03-01T09:00:30-02','Mon, 01 Sep 2025 12:00:30 +0000','01 Sep 2025 12:00 UT','2025-W01-3T12:34:56Z','2025-01-01X12:00:00+00:00:30.5','2025-13-01','01.09.2025','0001-01-01T00:00:00+01:00','9999-12-31T23:59:59-01:00','٢٠٢٥-٠٣-٠١T٠٩:٣٠Z',False,[],True,'Mon,01 Sep 2025 12:00:30 +0000','Mon,\x1c01\x1cSep\x1c2025\x1c12:00:30\x1c+0000','2025-03-01T09:00:30.١٢٣٤٥٦٧Z']
    for i,value in enumerate(times):yield dict(case=f'payout_time_{i}',op='payout_time',payout=dict(base,payout_occurred_at=value))
    for case,changes in [('valid',{}),('one_msat',dict(payout_amount=1)),('huge',dict(payout_amount=10**120+1)),('zero',dict(payout_amount=0,payout_fiat_value='bad')),('bad_amount',dict(payout_amount=[1])),('negative_value',dict(payout_fiat_value='-1')),('zero_value',dict(payout_fiat_value='0')),('empty_value',dict(payout_fiat_value='')),('bad_value',dict(payout_fiat_value='bad',payout_occurred_at='bad')),('nan',dict(payout_fiat_value='NaN')),('snan',dict(payout_fiat_value='sNaN')),('inf',dict(payout_fiat_value='Infinity')),('negative_inf',dict(payout_fiat_value='-Infinity')),('overflow',dict(payout_fiat_value='1e999')),('derived_overflow',dict(payout_amount=1,payout_fiat_value='1e308')),('value_before_time',dict(payout_fiat_value='Infinity',payout_occurred_at='bad')),('value_before_base',dict(payout_fiat_value='Infinity')),('bad_time',dict(payout_occurred_at='bad')),('legacy',dict(payout_occurred_at=times[7])),('fallback_id',dict(id='',payout_external_id=0)),('external',dict(payout_external_id='known',payout_asset='StraßeΣİ')),('denominator_rounding',dict(payout_amount=int('12345678901234567890123456789012345678901234567890123456789012345678901'),payout_fiat_value='123456789.50000000000000000000000005'))]:
        yield dict(case='payout_settlement_'+case,op='payout_settlement',observation=obs,payout=dict(base,**changes),rows={'source':row})
    yield dict(case='payout_settlement_denominator_rounding_priced',op='payout_settlement',observation=obs,
               payout=dict(base,payout_amount=int('123456789012345678901234567890123'),payout_fiat_value='423456789.50000000000000000000000005e10'),rows={'source':row})
    for case,value in [('exact','1.000000000000000000000000000000005'),('empty',''),('negative','-1'),('zero','-0.00'),('bad','bad'),('nan','NaN12'),('inf','Infinity'),('snan','sNaN'),('overflow','1e999'),('derived_overflow','1e308')]:
        yield dict(case='payout_price_'+case,op='payout_price',payout=dict(payout_fiat_value=value),amount=3,row=dict(id='projected',fiat_rate=9))
    for amount in (0,-1):
        yield dict(case=f'payout_price_amount_{amount}',op='payout_price',payout=dict(payout_fiat_value='bad'),amount=amount,row=dict(id='projected'))
    def decision(hash='s',start=0,end=1,**kw):return dict(source=dict(observation_hash=hash,start_msat=start,end_msat=end),**kw)
    for case,kw in [('ordinary',{}),('component',dict(component_id='c',reason='reviewed_custody_component')),('empty_component',dict(component_id='',reason='reviewed_custody_component')),('refund',dict(component_id='c',reason='reviewed_custody_component',transfer_kind='swap-refund')),('native',dict(component_id='c',reason='native_graph'))]:
        for mode in ('atomic','pair'):
            key=dict(atomic_bundle_id='bundle') if mode=='atomic' else dict(selected_claim_id='pair:p:suffix')
            ds=[decision(**key,**kw),decision(start=1,end=4,destination_kind='fee',**key),decision(start=4,end=5,**key,**kw),decision(hash='t',**key,**kw)]
            yield dict(case=f'fees_{case}_{mode}',op='fees',decisions=ds,observations={'s':dict(quantity_hash='s',fee_msat=7),'t':dict(quantity_hash='t',fee_msat=1)},actions=[dict(kind='move',decision=0),dict(kind='move',decision=2),dict(kind='move',decision=3),dict(kind='reviewed',decision=1),dict(kind='raw',decision=0)])
    for case,key in [('unbundled',{}),('empty_atomic_pair',dict(atomic_bundle_id='',selected_claim_id='pair:')),('atomic_precedence',dict(atomic_bundle_id='other',selected_claim_id='pair:p:tail')),('not_pair',dict(selected_claim_id='pairs:p'))]:
        ds=[decision(**key),decision(start=1,end=2,destination_kind='fee',selected_claim_id='pair:p:fee')]
        yield dict(case='fees_'+case,op='fees',decisions=ds,observations={'s':dict(quantity_hash='s',fee_msat=1)},actions=[dict(kind='reviewed',decision=1),dict(kind='raw',decision=0),dict(kind='move',decision=0),dict(kind='reviewed',decision=1)])
    ds=[decision(atomic_bundle_id='b'),decision(start=1,end=2,destination_kind='fee',atomic_bundle_id='b'),decision(start=2,end=10**120+3,destination_kind='fee',atomic_bundle_id='b'),decision(start=10**120+3,end=10**120+4,atomic_bundle_id='b')]
    yield dict(case='fees_multi_payout_exact',op='fees',decisions=ds,observations={'s':dict(quantity_hash='s',fee_msat=10**150+1)},actions=[dict(kind='move',decision=0),dict(kind='move',decision=3),dict(kind='reviewed',decision=1),dict(kind='reviewed',decision=2)])
