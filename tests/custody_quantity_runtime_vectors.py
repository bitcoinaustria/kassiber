"""Capture runtime operands at the Python boundary, independently of Rust."""
from dataclasses import fields
from functools import wraps
from pathlib import Path
import json
import unittest
from unittest.mock import patch
from tests.custody_snapshot import to_jsonable
from tests.custody_quantity_composition_vectors import outcome

PATH = Path(__file__).parent / 'fixtures/custody_quantity_runtime.json'


def restore(value):
    from kassiber.core import custody_evidence, custody_quantity, custody_quantity_runtime, custody_gap_holds, custody_gaps
    if isinstance(value, list):
        return tuple(restore(v) for v in value)
    if not isinstance(value, dict):
        return value
    if '$enum' in value:
        return custody_quantity.ClaimPriority(value['value'])
    if '$type' in value:
        cls = next(getattr(m, value['$type']) for m in (custody_evidence, custody_quantity, custody_quantity_runtime, custody_gap_holds, custody_gaps) if hasattr(m, value['$type']))
        return cls(**{f.name: restore(value[f.name]) for f in fields(cls)})
    return {k: restore(v) for k, v in value.items()}


def evaluate(v):
    from kassiber.core import custody_quantity_runtime as runtime
    if v['op'] == 'tax_eligibility':
        return outcome(lambda: runtime._tax_eligibility(restore(v['canonical']), restore(v['projection']), restore(v['issues']), v['rows']))
    if v['op'] == 'gap_holds':
        return outcome(lambda: runtime._gap_candidate_holds_and_issues(restore(v['canonical']), ignored_transaction_ids=v['ignored'], dismissed_fingerprints=v['dismissed'], search_result=restore(v['search'])))
    if v['op'] == 'component_runtime':
        return outcome(lambda: runtime._component_claims_and_issues(v['components'],restore(v['canonical']),v['evidence']))
    if v['op'] == 'runtime':
        return outcome(lambda: runtime.build_canonical_quantity_state(v['rows'], canonical_input=restore(v['canonical']), gap_search_result=restore(v['search']), interpreter_claims=restore(v['claims']), effective_components=v['components'], native_evidence=v['native'], interpreter_blockers=v['blockers'], ignored_gap_transaction_ids=v['ignored'], component_evidence_snapshots=v['evidence'], dismissed_gap_fingerprints=v['dismissed']))
    raise AssertionError(v['op'])


def vectors():
    from tests import test_custody_quantity_runtime as pins
    from kassiber.core import custody_quantity_runtime as runtime
    cases = []
    for cls in (pins.CustodyQuantityRuntimeTests, pins.CustodyQuantityStoreTests):
        for name in unittest.defaultTestLoader.getTestCaseNames(cls):
            build_original = runtime.build_canonical_quantity_state
            @wraps(build_original)
            def build_capture(rows, **kwargs):
                from kassiber.core.custody_evidence import enriched_quantity_rows, build_canonical_quantity_input
                if kwargs.get('canonical_input') is None:
                    rows = enriched_quantity_rows(rows)
                    kwargs['canonical_input'] = build_canonical_quantity_input(rows)
                v = dict(op='runtime', rows=to_jsonable(rows, redact_config=False), canonical=to_jsonable(kwargs['canonical_input'], redact_config=False), search=to_jsonable(kwargs['gap_search_result'], redact_config=False))
                for field, key, default in [('claims','interpreter_claims',()),('components','effective_components',()),('native','native_evidence',()),('blockers','interpreter_blockers',()),('ignored','ignored_gap_transaction_ids',()),('evidence','component_evidence_snapshots',None),('dismissed','dismissed_gap_fingerprints',{})]:
                    v[field] = to_jsonable(kwargs.get(key,default), redact_config=False)
                result = build_original(rows, **kwargs)
                cases.append(dict(case=name + ':' + str(len(cases)), **v, expected=to_jsonable(result, redact_config=False)))
                return result
            original = runtime._tax_eligibility
            gap_original = runtime._gap_candidate_holds_and_issues
            def gap_capture(canonical, **kwargs):
                v = dict(op='gap_holds', canonical=to_jsonable(canonical, redact_config=False), ignored=to_jsonable(sorted(kwargs['ignored_transaction_ids']), redact_config=False), dismissed=kwargs['dismissed_fingerprints'], search=to_jsonable(kwargs['search_result'], redact_config=False))
                cases.append(dict(case=name + ':' + str(len(cases)), **v, **outcome(lambda: gap_original(canonical, **kwargs))))
                return gap_original(canonical, **kwargs)
            def capture(canonical, projection, issues, rows):
                v = dict(op='tax_eligibility', canonical=to_jsonable(canonical, redact_config=False), projection=to_jsonable(projection, redact_config=False), issues=to_jsonable(issues, redact_config=False), rows=to_jsonable(rows, redact_config=False))
                cases.append(dict(case=name + ':' + str(len(cases)), **v, **outcome(lambda: original(canonical, projection, issues, rows))))
                return original(canonical, projection, issues, rows)
            with patch.object(runtime, '_tax_eligibility', capture), patch.object(runtime, '_gap_candidate_holds_and_issues', gap_capture), patch.object(runtime, 'build_canonical_quantity_state', build_capture), patch.object(pins, '_build_canonical_quantity_state', build_capture):
                result = unittest.TestResult()
                cls(name).run(result)
                assert not result.errors and not result.failures, (name, result.errors, result.failures)
    cases.extend(controls(cases))
    return cases

def controls(cases):
    from copy import deepcopy
    result=[]
    base=next(c for c in cases if c['op']=='gap_holds' and c['expected'][0] and c['search']['accounting_candidates'] and not c['ignored'])
    def add(name, template, mutate):
        v={k:deepcopy(x) for k,x in template.items() if k not in ('expected','error','case','message','details')}
        mutate(v)
        result.append(dict(case='control:'+name,**v,**evaluate(v)))
    def candidate(v): return v['search']['accounting_candidates'][0]
    def leg(v,direction): return next(o for e in v['canonical']['events'] for o in e['legs'] if o['direction']==direction)
    for field in ('source_ids','return_ids'):
        add(field+'_empty',base,lambda v,f=field:candidate(v).update({f:[]}))
        add(field+'_duplicate',base,lambda v,f=field:candidate(v).update({f:candidate(v)[f]*2}))
        add(field+'_unordered',base,lambda v,f=field:candidate(v).update({f:['z','a']}))
        add(field+'_missing',base,lambda v,f=field:candidate(v).update({f:['missing']}))
    for field,value in [('profile_id',''),('profile_id','other'),('gap_id','wrong'),('asset','ETH'),('source_wallet_ids',[]),('destination_wallet_ids',[])]:
        add('candidate_'+field+str(value),base,lambda v,f=field,x=value:candidate(v).update({f:x}))
    for field in ('source_total_msat','source_fee_msat','source_debit_msat','return_total_msat','retained_msat','residual_msat','excess_msat','coverage_ppm'):
        add('quantity_'+field,base,lambda v,f=field:candidate(v).update({f:candidate(v)[f]+1}))
    add('not_eligible',base,lambda v:candidate(v).update(promotion_eligible=False))
    for f,x in [('transaction_id','other'),('anchor_transaction_id',''),('wallet_id',''),('quantity_hash','bad'),('evidence_detail_hash','bad'),('asset',''),('direction','inbound'),('amount_msat',-1),('fee_msat',-1),('amount_msat',True)]:
        add('observation_'+f+str(x),base,lambda v,f=f,x=x:leg(v,'outbound').update({f:x}))
    for f,x in [('profile_id','other'),('asset','ETH')]:
        add('return_'+f,base,lambda v,f=f,x=x:leg(v,'inbound').update({f:x}))
    for f,x in [('chain','liquid'),('network','test')]:
        add('return_'+f,base,lambda v,f=f,x=x:leg(v,'inbound')['event_key'].update({f:x}))
    add('ignored_return',base,lambda v:v.update(ignored=candidate(v)['return_ids']))
    add('duplicate_candidates',base,lambda v:v['search']['accounting_candidates'].extend(deepcopy(v['search']['accounting_candidates'])))
    add('capacity_missing',base,lambda v:v['search'].update(search_complete=False,blocking_source_ids=['missing']))
    add('capacity_inbound',base,lambda v:v['search'].update(search_complete=False,blocking_source_ids=candidate(v)['return_ids']))
    add('capacity_ignored',base,lambda v:(v['search'].update(search_complete=False,blocking_source_ids=candidate(v)['source_ids']),v.update(ignored=candidate(v)['source_ids'])))
    add('capacity_with_candidate',base,lambda v:v['search'].update(search_complete=False,blocking_source_ids=candidate(v)['source_ids']))
    runtime=next(c for c in cases if c['op']=='runtime' and c['rows'])
    for detail in ('{broken','[]','null','{"nested": [1, true]}','{"x":NaN,"y":Infinity,"z":-Infinity}',None,7):
        add('blocker_detail_'+str(detail),runtime,lambda v,d=detail:v.update(blockers=[dict(transaction_id=v['rows'][0]['id'],reason='blocked',detail_json=d)]))
    for reason in (None,'ſtraße'):
        add('blocker_reason_'+str(reason),runtime,lambda v,r=reason:v.update(blockers=[dict(transaction_id='',reason=r,detail_json='{}')]))
    tax=next(c for c in cases if c['op']=='tax_eligibility' and c['case'].startswith('test_basis_barrier') and not c['issues'])
    def tax_issue(ids,at='2025-01-01T00:00:00Z',asset=None,kind='unresolved_quantity'):
        return dict(**{'$type':'QuantityIssue'},issue_id='control',issue_type=kind,state='conflicting',asset=asset,amount_msat=None,occurred_at=at,transaction_ids=ids,reason='control',details={})
    for ids in (['gap','late'],['late','gap']):
        add('earliest_barrier_'+str(ids),tax,lambda v,ids=ids:v.update(issues=[tax_issue([tx]) for tx in ids]))
    add('different_pool_barrier_times',tax,lambda v:v.update(issues=[tax_issue(['gap']),tax_issue(['other-profile'],at='2026-01-01T00:00:00Z')]))
    add('unknown_untimed_issue',tax,lambda v:v.update(issues=[tax_issue(['missing'],at='')]))
    add('unknown_unscoped_issue',tax,lambda v:v.update(issues=[tax_issue(['missing'])]))
    add('unknown_asset_scoped_issue',tax,lambda v:v.update(issues=[tax_issue(['missing'],asset='BTC')]))
    add('unknown_asset_falls_back',tax,lambda v:v.update(issues=[tax_issue(['missing'],asset='ETH')]))
    for network in ('liquidv1','liquidtestnet','elementsregtest'):
        def rejected(v,network=network):
            v['rows']['rejected']=dict(id='rejected',profile_id='profile-one',asset='LBTC',wallet_kind='liquid',raw_json=json.dumps(dict(chain='liquid',network=network)))
            v['issues']=[tax_issue(['rejected'])]
        add('rejected_pool_'+network,tax,rejected)
    for kind in ('native_audit_evidence_invalid','custody_gap_review_hold','custody_gap_hold_compile_failed'):
        add('direct_block_'+kind,tax,lambda v,kind=kind:v.update(issues=[tax_issue(['gap'],kind=kind)]))
    def same_time(v):
        for event in v['canonical']['events']:
            for o in event['legs']:
                if o['transaction_id']=='late':o['occurred_at']='2025-01-01T00:00:00Z'
        v['issues']=[tax_issue(['gap'])]
    add('equal_time_distinct_event',tax,same_time)
    compiler=json.loads(Path('tests/fixtures/custody_quantity_composition.json').read_text())
    canonical_template=runtime['canonical']
    for c in compiler:
        if c['op']!='reviewed':continue
        obs=c['observations']
        if isinstance(obs,dict):obs=list(obs.items())
        canonical=deepcopy(canonical_template)
        event=deepcopy(canonical['events'][0])
        event.update(legs=[o for _,o in obs],observation_aliases=[[tx,o['quantity_hash']] for tx,o in obs],evidence_snapshots=[])
        canonical.update(events=[event],rejected_events=[])
        v=dict(op='component_runtime',components=[c['component']],canonical=canonical,evidence=None)
        result.append(dict(case='reviewed:'+c['case'],**v,**evaluate(v)))
    return result


if __name__ == '__main__':
    PATH.write_text(json.dumps(vectors(), separators=(',', ':')) + '\n')
