"""Finalized-row primitive parity and enabled whole-stage admission."""
from collections import Counter
from copy import deepcopy
import json
import subprocess

import pytest

from tests.custody_projection_primitives import PATH, activity, operands, scenario, vectors


@pytest.fixture(scope='module')
def projection_driver():
    built=subprocess.run(['cargo','build','--offline','--locked','-p','kassiber-custody','--example','projection-primitives','--message-format=json'],capture_output=True,text=True,check=True)
    binary=next(record['executable'] for line in built.stdout.splitlines() if (record:=json.loads(line)).get('executable'))
    def invoke(inputs):
        completed=subprocess.run([binary],input=''.join(json.dumps(v)+'\n' for v in inputs),capture_output=True,text=True,check=True)
        result=[json.loads(line) for line in completed.stdout.splitlines()]
        assert len(result)==len(inputs)
        return result
    return invoke


def test_projection_frozen_python_vectors(projection_driver):
    frozen=json.loads(PATH.read_text())
    assert frozen==vectors()
    priced_names={'slice_rounding_denominator_priced', 'payout_settlement_denominator_rounding_priced'}
    priced={v['case']:v for v in frozen if v['case'] in priced_names}
    assert set(priced)==priced_names
    assert all('expected' in v for v in priced.values()), 'denominator rounding requires admitted prices'
    assert projection_driver(frozen)==[{k:v[k] for k in ('expected','error','message') if k in v} for v in frozen]


def distinct_results(driver, inputs):
    keys = [json.dumps(v, sort_keys=True) for v in inputs]
    unique = dict(zip(keys, inputs))
    outputs = dict(zip(unique, driver(list(unique.values()))))
    return [outputs[key] for key in keys]


def assert_activity(counts, controls):
    assert counts and all(value or case in controls for case,value in counts.items()), 'vacuous projection scenario'


def test_projection_primitive_vacuous_is_rejected():
    with pytest.raises(AssertionError, match='vacuous'):
        assert_activity({'vacuous': activity({'expected': None})}, set())


def test_projection_duplicate_operands_evaluated_once():
    calls = []
    def invoke(values):
        calls.extend(values)
        return [{'expected': True} for _ in values]
    assert len(distinct_results(invoke, [{'op': 'control'}] * 3)) == 3
    assert calls == [{'op': 'control'}]


def test_projection_generated_counts_both_engines(projection_driver):
    counts={engine:Counter() for engine in ('Python','Rust')}
    inputs=[];expected=[];names=[]
    for i in range(len(list(operands()))):
        seen=set()
        for variant in range(20):
            case,values,results=scenario(i,variant)
            key=json.dumps(values,sort_keys=True)
            if key in seen:continue
            seen.add(key)
            inputs.extend(values);expected.extend(results);names.append(case)
            counts['Python'][case]+=sum(map(activity,results))
    actual=distinct_results(projection_driver,inputs)
    assert actual==expected
    assert json.dumps(actual,sort_keys=True)==json.dumps(expected,sort_keys=True)
    for i,case in enumerate(names):counts['Rust'][case]+=activity(actual[i])
    # Counts belong only to each seed's distinct inputs, including refusals.
    # Explicit empty-success controls have zero activity; no companion rescues them.
    manifest=json.loads(PATH.with_name('custody_projection_counts.json').read_text())
    assert_activity(counts['Python'], {k for k,v in manifest.items() if not v})
    assert counts['Python']==counts['Rust']==manifest
    assert set(manifest)=={v['case'] for v in operands()}
    assert {k for k,v in manifest.items() if not v}=={'payout_settlement_zero_value', 'payout_settlement_zero', 'payout_valid_missing', 'payout_valid_too_large', 'payout_time_1', 'payout_valid_zero', 'payout_time_0', 'payout_settlement_negative_value', 'payout_valid_negative', 'payout_settlement_empty_value', 'payout_valid_inclusive', 'payout_valid_inbound'}
    # Dictionary insertion order is not a row lookup priority rule.
    shuffled=deepcopy(inputs)
    for v in shuffled:
        if isinstance(v.get('rows'),dict):v['rows']=dict(reversed(list(v['rows'].items())))
    assert json.dumps(distinct_results(projection_driver,shuffled),sort_keys=True)==json.dumps(expected,sort_keys=True)
    print('projection primitive active counts: '+json.dumps(dict(counts=counts, distinct_inputs=len({json.dumps(v,sort_keys=True) for v in inputs})),sort_keys=True))


def test_projection_stage_is_enabled():
    from tests.custody_engine_compare import rust_results
    from tests.custody_gap_generator import snapshot
    stages,skipped=rust_results(snapshot(0))
    assert 'finalized_projection' in stages
    assert skipped==()
