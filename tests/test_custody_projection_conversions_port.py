"""Conversion metadata parity, refusals and distinct seed activity."""
from collections import Counter
from copy import deepcopy
import json

import pytest

from tests.custody_projection_conversions import PATH, evaluate, operands, scenario
from tests.test_custody_projection_port import projection_driver  # noqa: F401


def assert_activity(counts, controls):
    assert counts and all(n or k in controls for k,n in counts.items()), 'vacuous scenario'


def test_projection_conversion_vacuous_is_rejected():
    with pytest.raises(AssertionError, match='vacuous'):
        assert_activity({'vacuous': 0}, set())


def test_projection_conversion_frozen(projection_driver):
    cases = [dict(v, **evaluate(v)) for v in operands()]
    assert cases == json.loads(PATH.read_text())
    assert projection_driver(cases) == [evaluate(v) for v in cases]


def test_projection_conversion_generated(projection_driver):
    inputs = [scenario(v,i) for v in operands() for i in range(20)]
    assert len({json.dumps(v,sort_keys=True) for v in inputs}) == len(inputs)
    expected = [evaluate(v) for v in inputs]
    actual = projection_driver(inputs)
    assert actual == expected
    counts = {engine: Counter() for engine in ('Python','Rust')}
    refusals = Counter()
    for v,py,rs in zip(inputs,expected,actual):
        for engine,result in [('Python',py),('Rust',rs)]:
            counts[engine][v['case']] += bool(result.get('expected') or result.get('error'))
        if 'error' in py:
            assert rs['error'] == py['error']; refusals[py['error']] += 1
    controls = {'wrong_amount','wrong_amount_high','wrong_direction','ambiguous','ambiguous_in','missing_leg','empty_pair','lazy_invalid_amount'}
    assert_activity(counts['Python'], controls)
    assert counts['Python'] == counts['Rust']
    assert {k for k,n in counts['Python'].items() if not n} == controls
    assert refusals == {'ValueError':40,'TypeError':40,'KeyError':20}
    assert counts['Python'] == json.loads(PATH.with_name('custody_projection_conversions_counts.json').read_text())
    shuffled = deepcopy(inputs)
    for v in shuffled:
        v['rows'].reverse()
        v['reviewed'] = [dict(reversed(list(p.items()))) for p in v['reviewed']]
    assert projection_driver(shuffled) == expected
    print(json.dumps(dict(counts=counts,refusals=refusals,distinct_inputs=len(inputs)),sort_keys=True))
