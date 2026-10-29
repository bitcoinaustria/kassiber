"""Selected rows remain private while the finalized stage is skipped."""
from collections import Counter
from copy import deepcopy
import json

import pytest

from tests.custody_projection_composition import PATH, evaluate, operands, scenario
from tests.test_custody_projection_port import projection_driver  # noqa: F401


def activity(result):
    # Refusals exercise their own error-code comparison, rather than borrowing
    # activity from a successful companion. Empty successes are vacuous.
    if 'error' in result:return 1
    result=result['expected']
    if isinstance(result,dict):
        return bool([r for r in result.get('rows', []) if not (r.get('journal_transaction_id') in {'other','target'} and r['id'].endswith(':inbound'))] or result.get('blocked') or result.get('blocked_transaction_ids') or result.get('barrier_held_receipts'))
    return bool(result)


def assert_activity(counts):
    assert counts and all(counts.values()), 'vacuous scenario: no seed activity'


def test_projection_composition_vacuous_is_rejected():
    with pytest.raises(AssertionError,match='vacuous'):
        assert_activity({'deliberately_vacuous':activity({'expected':{'rows':[]}})})


def test_projection_composition_frozen(projection_driver):
    cases=[dict(v,**evaluate(v)) for v in operands()]
    assert cases==json.loads(PATH.read_text())
    assert projection_driver(cases)==[{k:v[k] for k in ('expected','error') if k in v} for v in cases]


def test_projection_composition_generated(projection_driver):
    inputs=[scenario(v,i) for v in operands() for i in range(20)]
    # Distinct raw operands only. Each seed owns its activity and refusals.
    assert len({json.dumps({k:x for k,x in v.items() if k != 'case'},sort_keys=True) for v in inputs})==len(inputs)
    expected=[evaluate(v) for v in inputs]
    actual=projection_driver(inputs)
    assert actual==expected
    assert json.dumps(actual,sort_keys=True)==json.dumps(expected,sort_keys=True)
    counts={engine:Counter() for engine in ('Python','Rust')}
    refusals=Counter()
    for v,py,rust in zip(inputs,expected,actual):
        counts['Python'][v['case']]+=activity(py)
        counts['Rust'][v['case']]+=activity(rust)
        if 'error' in py:
            assert rust['error']==py['error'];refusals[py['error']]+=1
    assert refusals, 'refusal comparison did not run'
    assert_activity(counts['Python']);assert_activity(counts['Rust'])
    assert counts['Python']==counts['Rust']
    manifest=json.loads(PATH.with_name('custody_projection_composition_counts.json').read_text())
    assert counts['Python']==manifest
    shuffled=deepcopy(inputs)
    for v in shuffled:v['rows']=dict(reversed(list(v['rows'].items())))
    assert projection_driver(shuffled)==expected
    print(json.dumps(dict(counts=counts,refusals=refusals,distinct_inputs=len(inputs)),sort_keys=True))


def test_projection_companion_receipts_cannot_supply_activity():
    from tests.custody_projection_composition import seed
    value = seed()
    value['decisions'] = []
    value['observations'][0]['fee_msat'] = 0
    result = evaluate(value)
    assert len(result['expected']['rows']) == 2
    with pytest.raises(AssertionError, match='vacuous'):
        assert_activity({'deliberately_vacuous_with_receipts': activity(result)})
