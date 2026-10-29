"""Partial finalized-row parity; complete projection remains skipped."""
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
    assert projection_driver(frozen)==[{k:v[k] for k in ('expected','error','message') if k in v} for v in frozen]


def test_projection_generated_counts_both_engines(projection_driver):
    counts={engine:Counter() for engine in ('Python','Rust')}
    inputs=[];expected=[];names=[]
    for i in range(len(list(operands()))):
        for variant in range(20):
            case,values,results=scenario(i,variant)
            inputs.extend(values);expected.extend(results);names.append(case)
            counts['Python'][case]+=sum(map(activity,results))
    actual=projection_driver(inputs)
    assert actual==expected
    assert json.dumps(actual,sort_keys=True)==json.dumps(expected,sort_keys=True)
    for i,case in enumerate(names):counts['Rust'][case]+=sum(map(activity,actual[i*2:i*2+2]))
    # Independent counts pin every scenario, including an active positive
    # companion for each refusal. No engine can silently drop a scenario.
    manifest=json.loads(PATH.with_name('custody_projection_counts.json').read_text())
    assert counts['Python']==counts['Rust']==manifest
    assert set(manifest)=={v['case'] for v in operands()}
    assert all(manifest.values())
    # Dictionary insertion order is not a row lookup priority rule.
    shuffled=deepcopy(inputs)
    for v in shuffled:
        if isinstance(v.get('rows'),dict):v['rows']=dict(reversed(list(v['rows'].items())))
    assert json.dumps(projection_driver(shuffled),sort_keys=True)==json.dumps(expected,sort_keys=True)
    print('projection primitive active counts: '+json.dumps(counts,sort_keys=True))


def test_projection_stage_still_skipped():
    from tests.custody_engine_compare import rust_results
    from tests.custody_gap_generator import snapshot
    stages,skipped=rust_results(snapshot(0))
    assert 'finalized_projection' not in stages
    assert skipped==('finalized_projection',)
