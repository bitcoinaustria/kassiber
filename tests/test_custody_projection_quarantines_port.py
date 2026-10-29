"""Full private projection comparison; snapshot stage admission remains separate."""
from collections import Counter
from copy import deepcopy
import json

import pytest

from tests.custody_projection_quarantines import PATH, evaluate, operands, scenario
from tests.test_custody_projection_port import projection_driver  # noqa: F401


def activity(result):
    if 'error' in result:
        return True
    return bool(result['expected']['rows'] or result['expected']['quarantines'])


def assert_activity(counts):
    assert counts and all(n or k == 'dependency_absent_row' for k,n in counts.items()), 'vacuous quarantine scenario'


def test_projection_quarantine_vacuous_is_rejected():
    with pytest.raises(AssertionError, match='vacuous'):
        assert_activity({'empty': activity({'expected': dict(rows=[], quarantines=[])})})


def test_projection_quarantine_frozen(projection_driver):
    cases = [dict(v, **evaluate(v)) for v in operands()]
    assert cases == json.loads(PATH.read_text())
    assert projection_driver(cases) == [evaluate(v) for v in cases]


def test_projection_quarantine_generated(projection_driver):
    inputs = [scenario(v,i) for v in operands() for i in range(20)]
    keys = [json.dumps({k:x for k,x in v.items() if k != 'case'}, sort_keys=True) for v in inputs]
    assert len(set(keys)) == len(inputs)
    expected = [evaluate(v) for v in inputs]
    actual = projection_driver(inputs)
    assert actual == expected
    counts = {engine: Counter() for engine in ('Python','Rust')}
    refusals = Counter()
    for v,py,rs in zip(inputs,expected,actual):
        for engine,result in [('Python',py),('Rust',rs)]:
            counts[engine][v['case']] += activity(result)
        if 'error' in py:
            assert rs['error'] == py['error']
            refusals[py['error']] += 1
    assert refusals
    assert_activity(counts['Python'])
    assert {k for k,n in counts['Python'].items() if not n} == {'dependency_absent_row'}
    assert counts['Python'] == counts['Rust']
    assert counts['Python'] == json.loads(PATH.with_name('custody_projection_quarantines_counts.json').read_text())
    shuffled = deepcopy(inputs)
    for v in shuffled:
        v['rows'] = dict(reversed(list(v['rows'].items())))
        v['blocked_transaction_ids'].reverse()
        v['issues'].reverse()
    assert projection_driver(shuffled) == expected
    print(json.dumps(dict(counts=counts,refusals=refusals,distinct_inputs=len(inputs)),sort_keys=True))
