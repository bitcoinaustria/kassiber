"""Independent CPython finalized-row fixtures and exact-width generated operands."""
from copy import deepcopy
import json
from pathlib import Path

from kassiber.core import custody_tax_projection as p
from kassiber.core.custody_quantity import QuantityObservation
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_snapshot import to_jsonable
from tests.test_custody_quantity import _observation, _row

PATH = Path(__file__).parent / 'fixtures/custody_projection_primitives.json'


def evaluate(v):
    try:
        op = v['op']
        from tests import custody_projection_money as money
        if op in money.OPS:
            return {'expected': to_jsonable(money.evaluate(v), redact_config=False)}
        if op == 'validate':
            p.FinalizedTaxProjection(tuple(v['rows']), tuple(v['pairs']), (), (), ())
            result = True
        elif op == 'ids':
            result = [p.tax_slice_row_id(v['hash'], v['start'], v['end'], v['side']),
                      p.tax_fee_row_id(v['hash'], v['side'], v['amount']),
                      p.direct_payout_row_id(v['payout_id'], v['side'])]
        else:
            obs = deepcopy(v['observation'])
            obs.pop('$type', None)
            from kassiber.core.custody_evidence import CanonicalEventKey
            event = obs['event_key']; event.pop('$type', None)
            obs['event_key'] = CanonicalEventKey(**event)
            observation = QuantityObservation(**obs)
            if op == 'base':
                result = p._base_row(observation, v['rows'])
            else:
                result = p._projected_fee_row(observation, v['amount'], v['rows'], side=v['side'])
        return {'expected': to_jsonable(result, redact_config=False)}
    except Exception as exc:
        return {'error': type(exc).__name__, **({'message': str(exc)} if v['op']=='validate' else {})}


def operands():
    good = {'id': 'projected', 'custody_finalized_tax_projection': True}
    def contract(case, rows, pairs=()):
        return dict(case=case, op='validate', rows=rows, pairs=list(pairs))
    yield contract('empty', [])
    yield contract('valid_pair', [good], [{'out':good,'in':good}])
    yield contract('truthy_marker', [dict(good,custody_finalized_tax_projection='yes')])
    yield contract('raw_before_pair', [{'id':'raw'}], [{'out':None}])
    yield contract('duplicate_before_raw', [good,good,{'id':'raw'}])
    for name, ident in [('missing',None), ('false',False), ('zero',0), ('empty_id','')]:
        yield contract(name, [dict(good,id=ident)])
    yield contract('duplicate_string_coercion',[dict(good,id=1),dict(good,id='1')])
    yield contract('container_id',[dict(good,id={'a':True})])
    yield contract('pair_shape',[good],[{'out':good,'in':[]}])
    yield contract('pair_absent',[good],[{'out':good,'in':dict(good,id='raw')}])
    yield contract('pair_missing_id',[good],[{'out':good,'in':{}}])
    # Pair membership checks only ids, not pair row object identity or markers.
    yield contract('pair_id_only',[good],[{'out':{'id':'projected'},'in':good}])
    yield dict(case='exact_ids',op='ids',hash='hash:ß',start=10**100+1,end=10**101+2,amount=10**110+3,side='move-out:Ω',payout_id='payout:é')
    yield dict(case='signed_ids',op='ids',hash='h',start=-1,end=0,amount=-7,side='',payout_id='')
    observation = _observation('source','wallet','outbound',100,fee_msat=7)
    row = _row('source','wallet','outbound',100,fee_msat=7)
    row.update(privacy_boundary='unresolved',fiat_value=50,fiat_value_exact='50.000',fiat_rate_exact='50000.1',extra={'keep':True})
    obs = to_jsonable(observation,redact_config=False)
    for case, rows in [('source',{ 'source': row }), ('rowless',{}), ('anchor',{'anchor':row}), ('empty_source',{'source':{},'anchor':row})]:
        o = dict(obs,anchor_transaction_id='anchor' if case in ('anchor','empty_source') else 'source')
        yield dict(case='base_'+case,op='base',observation=o,rows=deepcopy(rows))
    yield dict(case='base_source_precedence',op='base',observation=dict(obs,anchor_transaction_id='anchor'),rows={'source':row,'anchor':dict(row,extra='fallback')})
    yield dict(case='fee_anchor',op='fee',observation=dict(obs,anchor_transaction_id='anchor'),rows={'anchor':row},amount=7,side='non-event-fee')
    yield dict(case='base_raw_exact',op='base',observation=dict(obs,occurred_at='2025-W01-3X12:34:56.123456+00:00'),rows={'source':dict(row,amount=10**150+1,fee=10**100+3,kind='StraßeΣİ')})
    for field in ('fiat_value', 'fiat_value_exact', 'fiat_rate', 'fiat_rate_exact'):
        for name, value in [('nonfinite', 'Infinity'), ('overflow', '1e999999999'), ('tiny', '1e-100000001'), ('below_precision', '9e-14'), ('precision_floor', '1e-13'), ('long_exponent', '1.' + '0' * 309), ('zero_long_exponent', '0e-999')]:
            yield dict(case=f'base_price_{field}_{name}', op='base', observation=obs,
                       rows={'source': dict(row, **{field: value})})
    for external, raw in [(None,None),('',[]),(False,''),('kept',{'x':1})]:
        yield dict(case='fallback_'+str(external),op='base',observation=obs,rows={'source':dict(row,external_id=external,raw_json=raw)})
    for authority, attribution in [(False,'exact'),(True,'shared'),(True,'exact')]:
        yield dict(case=f'fee_privacy_{authority}_{attribution}',op='fee',observation=dict(obs,authoritative_chain_observation=authority,fee_attribution=attribution),rows={'source':row},amount=10**90+1,side='fee-only')
    # Original closed provenance is captured before the synthetic id and zero principal.
    raw = dict(txid='a'*64,chain='bitcoin',network='main',vin=[
        dict(txid='B'*64,vout=10**100),dict(txid='c'*64,vout=True),dict(txid='d'*64,vout=-1),dict(txid='e'*64,vout='0'),dict(txid='f'*64,vout=0.0),
        dict(txid='b'*64,vout=0)],tx={'vin':[dict(txid='1'*64,vout=0)]},ownership_graph={'vin':[dict(txid='2'*64,vout=1),None]})
    chain = dict(row,external_id='a'*64,raw_json=json.dumps(raw),observation_authority_version=1)
    chain['observation_graph_hash']=canonical_graph_hash(chain['raw_json'])
    chain['observation_quantity_hash']=canonical_observed_quantity_hash(chain)
    for case, changes in [('closed',{}),('stale_quantity',{'amount':101}),('stale_graph',{'raw_json':'{}'}),('unsupported',{'observation_authority_version':2}),('missing',{'observation_authority_version':None}),('unicode_version',{'observation_authority_version':'١'}),('synthetic',{'id':'custody-tax:old'}),('invalid_amount',{'amount':'bad'}),('amount_type',{'amount':[1]}),('fee_type',{'fee':{'bad':1}}),('graph_before_amount',{'raw_json':'{}','amount':[1]})]:
        yield dict(case='chain_'+case,op='fee',observation=obs,rows={'source':dict(chain,**changes)},amount=7,side='fee-only')


    from tests.custody_projection_money import operands as money_operands
    yield from money_operands()


def vectors():
    return [dict(v,**evaluate(v)) for v in operands()]


_evaluated = {}


def evaluate_distinct(value):
    key = json.dumps(value, sort_keys=True)
    if key not in _evaluated:
        _evaluated[key] = evaluate(value)
    return deepcopy(_evaluated[key])


def scenario(i,variant):
    base = list(operands())[i]
    v = {k:deepcopy(x) for k,x in base.items() if k!='case'}
    if v['op']=='ids':
        for key in ('start','end','amount'): v[key]*=(1,3,10**60+1,10**120+7)[variant%4]
    elif v['op']=='fee':
        v['amount']*=(1,3,10**60+1,10**120+7)[variant%4]
    if v['op'] in ('base','fee'):
        v['observation']['wallet_id']+=f':Σ:{variant}'
        v['observation']['occurred_at']+=f'#{variant}'
        for row in v['rows'].values():
            if row: row['projection_fixture_variant']=variant
    if v['op']=='slice':
        factor=(1,3,10**60+1,10**120+7)[variant%4]
        v['quantity']['start_msat']*=factor;v['quantity']['end_msat']*=factor;v['fee']*=factor
    if v['op'] in ('slice','payout_settlement','payout_price','payout_valid'):
        factor=(1,3,10**60+1,10**120+7)[variant%4]
        if v['op']=='slice':
            for row in v['rows'].values():
                if type(row.get('amount')) is int:row['amount']*=factor
        elif v['op']=='payout_settlement':
            amount=v['payout'].get('payout_amount')
            if type(amount) is int:v['payout']['payout_amount']*=factor
        elif v['op']=='payout_price':v['amount']*=factor
        else:
            for row in v['rows'].values():
                for key in ('amount','fee'):
                    if type(row.get(key)) is int:row[key]*=factor
            for record in v['records']:
                if type(record.get('out_amount')) is int:record['out_amount']*=factor
    if v['op']=='payout_time' and isinstance(v['payout'].get('payout_occurred_at'),str) and v['payout']['payout_occurred_at']:
        v['payout']['payout_occurred_at']='\u2003'+v['payout']['payout_occurred_at']+'\u2003'
    if v['op']=='fees':
        factor=(1,3,10**60+1,10**120+7)[variant%4]
        for d in v['decisions']:
            d['source']['start_msat']*=factor;d['source']['end_msat']*=factor
        for o in v['observations'].values():o['fee_msat']*=factor
    return base['case'],[v],[evaluate_distinct(v)]


def activity(result):
    if 'error' in result:return 1
    value=result['expected']
    if value is None or value=={}:return 0
    return len(value) if isinstance(value,list) else 1


if __name__=='__main__':
    PATH.write_text(json.dumps(vectors(),ensure_ascii=True,sort_keys=True)+'\n')
