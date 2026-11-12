"""Independent Python compiler inputs/outputs captured from behavior pins."""
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from tests.custody_snapshot import to_jsonable
from copy import deepcopy

PATH = Path(__file__).parent / 'fixtures/custody_quantity_composition.json'


def outcome(call):
    try:
        return {'expected': to_jsonable(call(), redact_config=False)}
    except Exception as exc:
        result = {'error': getattr(exc, 'code', type(exc).__name__)}
        if hasattr(exc, 'code'):
            result.update(message=str(exc), details=exc.details)
        return result


def vectors():
    from tests import test_custody_quantity_runtime as pins
    from kassiber.core import custody_quantity_runtime as runtime
    cases = []
    for cls in (pins.CustodyQuantityRuntimeTests, pins.CustodyQuantityStoreTests):
      for name in unittest.defaultTestLoader.getTestCaseNames(cls):
          originals = {op: getattr(runtime, function) for op, function in (
              ('reviewed', 'compile_component_quantity_claims'),
              ('native', 'compile_verified_native_claims'))}
          def reviewed(component, observations):
              value = dict(op='reviewed', component=to_jsonable(component, redact_config=False), observations=to_jsonable(observations, redact_config=False))
              value.update(outcome(lambda: originals['reviewed'](component, observations)))
              cases.append(dict(case=name + ':' + str(len(cases)), **value))
              return originals['reviewed'](component, observations)
          def native(canonical, audit, **kwargs):
              value = dict(op='native', canonical=to_jsonable(canonical, redact_config=False), audit=to_jsonable(audit, redact_config=False), excluded=to_jsonable(sorted(kwargs.get('component_transaction_ids', ())), redact_config=False), reserved=to_jsonable(kwargs.get('reserved_source_msat') or {}, redact_config=False))
              value.update(outcome(lambda: originals['native'](canonical, audit, **kwargs)))
              cases.append(dict(case=name + ':' + str(len(cases)), **value))
              return originals['native'](canonical, audit, **kwargs)
          with patch.object(runtime, 'compile_component_quantity_claims', reviewed), patch.object(runtime, 'compile_verified_native_claims', native):
              result = unittest.TestResult()
              cls(name).run(result)
              assert not result.errors and not result.failures, (name, result.errors, result.failures)
    from tests.custody_reconciliation_vectors import operands
    from kassiber.core.custody_component_claims import compile_component_quantity_claims
    for case, op, value in operands():
        if op == 'reviewed':
            cases.append(dict(case='reconciliation:' + case, op=op,
                component=to_jsonable(value['component'], redact_config=False),
                observations=to_jsonable(value['observations'], redact_config=False),
                **outcome(lambda: compile_component_quantity_claims(value['component'], value['observations']))))
    cases.extend(additional(cases))
    for value in cases:
        if value['op'] == 'reviewed':
            value['observations'] = [[k, v] for k, v in value['observations'].items()]
    return cases


def additional(cases):
    from tests.custody_quantity_composition_generator import evaluate
    result = []
    conversion = next(c for c in cases if c['op'] == 'reviewed' and c['component'].get('conservation_mode') == 'conversion')
    def add(name, base, mutate):
        v = {k: deepcopy(value) for k, value in base.items() if k not in ('expected', 'error', 'case', 'message', 'details')}
        mutate(v)
        result.append(dict(case='control:' + name, **v, **evaluate(v)))
    add('conversion_defaults', conversion, lambda v: v['component'].update(economic_terms=[], conversion_policy=None, component_type=None))
    add('conversion_inferred', conversion, lambda v: v['component'].update(allocations=[]))
    def same_asset(v):
        c = v['component']; a = c['allocations'][0]
        source = next(o for o in v['observations'].values() if o['direction']=='outbound')
        target = next(o for o in v['observations'].values() if o['direction']=='inbound')
        target['asset'] = source['asset']; target['amount_msat'] = a['source_amount_msat']; a['sink_amount_msat'] = a['source_amount_msat']
    add('conversion_native_residual', conversion, same_asset)
    def partial(v):
        a = v['component']['allocations'][0]; a['source_amount_msat'] //= 2
    add('conversion_high_slice', conversion, partial)
    def payout(v):
        c=v['component']; sink=next(l for l in c['legs'] if l['role']=='destination')
        sink.update(transaction_id=None,anchor_transaction_id=None)
        c['economic_terms'][0].update(term_kind='direct_swap_payout',payout_amount_msat=0,payout_asset='',payout_occurred_at='',payout_fiat_value_exact='10.001',payout_external_id='external',counterparty='reviewed',swap_fee_kind='service',review_notes='notes',confidence_at_review='exact',review_source='manual')
    add('conversion_direct_payout', conversion, payout)
    def fee(v):
        c=v['component']; c['legs'][1]['role']='fee'; c['allocations'][0]['sink_amount_msat']=c['allocations'][0]['source_amount_msat']

    def conversion_retained(v):
        sink=v['component']['legs'][1]
        sink.update(role='retained',transaction_id=None,anchor_transaction_id=None,notes='reviewed_residual:retained_custody',wallet_id='retained')
        v['component']['allocations'][0]['sink_amount_msat']=v['component']['allocations'][0]['source_amount_msat']
    add('conversion_retained_constructor_refusal',conversion,conversion_retained)
    add('conversion_fee', conversion, fee)
    add('conversion_fee_unequal', conversion, lambda v: v['component']['legs'][1].update(role='fee'))
    add('conversion_unobserved_source', conversion, lambda v: v['component']['legs'][0].update(transaction_id=None,anchor_transaction_id=None))
    add('conversion_over_capacity', conversion, lambda v: v['component']['allocations'][0].update(source_amount_msat=10**90))
    add('conversion_multileg_inference', conversion, lambda v: v['component'].update(allocations=[],legs=v['component']['legs']*2))
    unfunded=next(c for c in cases if c['case']=='reconciliation:unfunded_route')
    add('duplicate_pending_allocation_ids',unfunded,lambda v:v['component']['allocations'].append(deepcopy(v['component']['allocations'][-1])))
    quantity=next(c for c in cases if c['op']=='reviewed' and c['case']=='reconciliation:recovered')
    for classification in ('external_payment','external_disposal','external_gift','external_loss'):
        def external(v, classification=classification):
            c=v['component']; c.update(component_type='manual_bridge', evidence_kind='custody_gap_review', evidence={'residual_classification':{'classification':classification}})
            c['legs'][1].update(role='external',notes='reviewed_residual:'+classification)
        add(classification,quantity,external)
    add('inferred_suspense_requires_allocations',quantity,lambda v:v['component'].update(allocations=[],legs=[dict(l,role='suspense') if l['role']=='destination' else l for l in v['component']['legs']]))
    for field,value in [('id',''),('effective_state','draft')]:
        add('required_'+field,quantity,lambda v,field=field,value=value:v['component'].update({field:value}))
    add('missing_leg_id',quantity,lambda v:v['component']['legs'][0].pop('id'))
    add('unicode_integer',quantity,lambda v:v['component']['allocations'][0].update(ordinal='٣'))
    native=next(c for c in cases if c['op']=='native' and c['audit'] and c.get('expected',{}).get('claims') and not c['audit'][0].get('virtual_source'))
    for field,value in [('out_id',''),('in_id',''),('out_anchor_transaction_id','missing'),('from_wallet_id','wrong'),('to_wallet_id',''),('asset','ß'),('crypto_received_msat',True),('crypto_received_msat',0),('crypto_fee_msat',-1),('crypto_sent_msat','1'),('occurred_at','different'),('pairing_source','unknown')]:
        add('native_'+field+'_'+repr(value),native,lambda v,field=field,value=value:v['audit'][0].update({field:value}))
    def implicit(v):
        source_id=v['audit'][0]['out_anchor_transaction_id']
        source_hash=next(hash for event in v['canonical']['events'] for tx,hash in event['observation_aliases'] if tx==source_id)
        for event in v['canonical']['events']:
            for o in event['legs']:
                if o['quantity_hash']==source_hash:
                    o['amount_msat']+=5
                    o['fee_attribution']='implicit_wallet_delta'
    add('native_implicit_residual',native,implicit)
    exact_target=next(c for c in cases if c['op']=='native' and c['case'].startswith('test_native_audit_reuses_exact_real_inbound'))
    def target_contradiction(v):
        for event in v['canonical']['events']:
            for o in event['legs']:
                if o['direction']=='inbound':o['amount_msat']+=1
    add('native_real_target_contradiction',exact_target,target_contradiction)
    def cross_rail(v):
        for o in v['observations'].values():
            if o['direction']=='inbound':o['event_key']['chain']='liquid';o['asset']='LBTC'
    add('reviewed_cross_rail',quantity,cross_rail)
    def negative_source(v):
        v['component']['allocations'][0]['source_amount_msat']=-1
    add('conversion_negative_source_slice',conversion,negative_source)
    def negative_target_cursor(v):
        same_asset(v)
        a=v['component']['allocations'][0]
        a.update(source_amount_msat=1,sink_amount_msat=-1)
        v['component']['allocations'].append(dict(a,id='next',source_amount_msat=1,sink_amount_msat=1))
    add('conversion_negative_target_cursor',conversion,negative_target_cursor)
    def fallback_residual(v):
        c=v['component'];c.update(component_type='manual_bridge',evidence_kind='custody_gap_review')
        c.pop('evidence',None)
        c['legs'][1].update(role='external',notes='reviewed_residual:external_gift')
    add('residual_leg_classification',quantity,fallback_residual)
    def fanout(v):
        a=v['audit'][0]
        amount=a['crypto_received_msat']; fee=a['crypto_fee_msat']
        a.update(crypto_received_msat=amount//2,crypto_sent_msat=amount//2+fee)
        other=dict(a,out_id='second-out',in_id='second-in',to_wallet_id='another-owned',crypto_received_msat=amount-amount//2,crypto_fee_msat=0,crypto_sent_msat=amount-amount//2)
        v['audit'].append(other)
    add('native_two_target_fanout',native,fanout)
    def mixed_group(v):
        fanout(v)
        v['audit'][0].update(pairing_source='recorded_fanout')
        v['audit'][0]['crypto_fee_msat']+=1
        v['audit'][0]['crypto_sent_msat']+=1
    add('native_mixed_invalid_source_group',native,mixed_group)
    add('native_unicode_identity',native,lambda v:v['audit'][0].update(in_id='in-ß-💰',to_wallet_id='owned-ß-💰'))

    add('native_duplicate_leg',native,lambda v:v['audit'].append(deepcopy(v['audit'][0])))
    add('native_missing_anchor_empty_asset',native,lambda v:v['audit'][0].update(out_anchor_transaction_id='missing',asset=''))
    add('native_excluded',native,lambda v:v.update(excluded=[v['audit'][0]['out_anchor_transaction_id']]))
    from kassiber.core.custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
    from tests.test_custody_quantity_runtime import _row, CustodyQuantityRuntimeTests
    time='2025-01-01T00:00:00Z'
    rows=[_row('virtual-target','chain-wallet','inbound',777,time,txid='ce'*32)]
    canonical=build_canonical_quantity_input(enriched_quantity_rows(rows))
    audit=CustodyQuantityRuntimeTests._native_audit(source_id='virtual-out',out_id='virtual-out',in_id='virtual-target',from_wallet='node',to_wallet='chain-wallet',received=777,fee=7,occurred_at=time,pairing_source='channel_lifecycle')
    audit.update(virtual_source=True,in_anchor_transaction_id='virtual-target')
    virtual=dict(op='native',canonical=to_jsonable(canonical,redact_config=False),audit=[audit],excluded=[],reserved={})
    add('virtual_source',virtual,lambda v:None)
    for field,value in [('out_id',''),('in_anchor_transaction_id','missing'),('from_wallet_id',''),('asset','USDT'),('to_wallet_id','wrong'),('crypto_received_msat',True),('crypto_fee_msat',None),('crypto_sent_msat',0),('occurred_at','wrong')]:
        add('virtual_invalid_'+field,virtual,lambda v,field=field,value=value:v['audit'][0].update({field:value}))
    add('virtual_target_contradiction',virtual,lambda v:v['audit'][0].update(crypto_received_msat=776,crypto_sent_msat=783))
    prior=deepcopy(virtual);prior['canonical']=evaluate(virtual)['expected']['canonical_input']
    add('virtual_existing_source',prior,lambda v:None)
    add('virtual_existing_contradiction',prior,lambda v:v['audit'][0].update(from_wallet_id='another-node'))
    rows=[_row('s'+str(i),'source-'+str(i),'outbound',100,time,txid='dd'*32) for i in range(3)]
    rows.append(_row('contradiction','z-contradiction','inbound',49,time,txid='dd'*32))
    audits=[]
    for source,wallet in [(0,'b-near'),(0,'z-contradiction'),(1,'b-near'),(1,'a-far'),(2,'a-far'),(2,'c-far-sibling')]:
        audits.append(CustodyQuantityRuntimeTests._native_audit(source_id='s'+str(source),out_id='out-'+str(len(audits)),in_id='in-'+str(len(audits)),from_wallet='source-'+str(source),to_wallet=wallet,received=50,fee=0,occurred_at=time))
    transitive=dict(op='native',canonical=to_jsonable(build_canonical_quantity_input(enriched_quantity_rows(rows)),redact_config=False),audit=audits,excluded=[],reserved={})
    add('native_transitive_target_invalidation',transitive,lambda v:None)
    for field,value in [('chain','lightning'),('native_namespace','wallet'),('native_event_id','A'*64),('native_event_id','short')]:
        def scope(v,field=field,value=value):
            for event in v['canonical']['events']:
                event['event_key'][field]=value
                for o in event['legs']:o['event_key'][field]=value
        add('native_scope_'+field+'_'+value,native,scope)
    def reservation(v,value):
        source_id=v['audit'][0]['out_anchor_transaction_id']
        hash=next(hash for event in v['canonical']['events'] for tx,hash in event['observation_aliases'] if tx==source_id)
        v['reserved']={hash:value}
    for value in (None,[],{},'bad','٠',True,-1,10**90):
        add('native_reservation_'+repr(value),native,lambda v,value=value:reservation(v,value))
    return result


if __name__ == '__main__':
    PATH.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')
