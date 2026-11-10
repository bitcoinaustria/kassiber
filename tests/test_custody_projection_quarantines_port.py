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
    return bool([r for r in result['expected']['rows'] if not (r.get('journal_transaction_id') in {'other','target'} and r['id'].endswith(':inbound'))] or result['expected']['quarantines'])


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


def test_projection_lossless_diagnostics_match_python():
    """Compare actual copied values and refusals through the CPython codec."""
    import subprocess

    built = subprocess.run(
        ['cargo', 'build', '--offline', '--locked', '-p', 'kassiber-custody',
         '--example', 'projection-lossless', '--message-format=json'],
        capture_output=True, text=True, check=True,
    )
    binary = next(record['executable'] for line in built.stdout.splitlines()
                  if (record := json.loads(line)).get('executable'))
    base = next(operands())
    diagnostics = [
        [dict(transaction_id='source', reason='specific', detail_json=dict(
            huge=10**90, nan=float('nan'), inf=float('inf'), negative=-float('inf'), zero=-0.0))],
        [dict(transaction_id='\ud800', reason='\udfff', detail_json={'\ud800': ['\udfff', '\ud800x']})],
        [dict(transaction_id='source', reason='z', detail_json='first'),
         dict(transaction_id='other', reason='a', detail_json='second'),
         dict(transaction_id='source', reason='z', detail_json='last')],
        [dict(transaction_id='source')],
        [dict(transaction_id='', reason='ignored')],
        [dict(transaction_id='\ud800', reason='r'), dict(transaction_id='\udfff', reason='r')],
    ]
    primary = [dict(base, interpreter_quarantines=items) for items in diagnostics]
    assert len({json.dumps(v, sort_keys=True) for v in primary}) == 6
    variants = primary + [dict(v, interpreter_quarantines=list(reversed(v['interpreter_quarantines']))) for v in primary]
    inputs = list({json.dumps(v, sort_keys=True): v for v in variants}.values())
    assert len(inputs) == 8  # Six primaries and two additional distinct orderings.
    expected = [evaluate(v) for v in inputs]
    requests = [dict(operands_json=json.dumps(base), diagnostics=v['interpreter_quarantines']) for v in inputs]
    result = subprocess.run([binary], input=''.join(json.dumps(v) + '\n' for v in requests),
                            capture_output=True, text=True, check=True)
    actual = [json.loads(line) for line in result.stdout.splitlines()]
    from tests.custody_snapshot import to_jsonable
    # Apply the independent comparison projection, including its explicit
    # nonfinite tags; signed zero and lone surrogates remain distinguishable.
    assert json.dumps(to_jsonable(actual, redact_config=False), sort_keys=True) == json.dumps(expected, sort_keys=True)
    assert expected[3] == actual[3] == {'error': 'KeyError'}
    assert all(activity(v) for v in expected)
    assert all(activity(v) for v in actual)
    assert_activity({str(i): int(activity(v)) for i, v in enumerate(actual)})
    print('lossless diagnostics: 8 distinct inputs, 7 successes, 1 compared KeyError')


def test_projection_ignored_interpreter_issue_preserves_admission():
    baseline = next(operands())
    candidate = deepcopy(baseline)
    candidate['issues'] = [dict(reason='transfer_fee_implausible',
        issue_type='custody_interpreter_blocked', transaction_ids=['source'],
        issue_id='ignored', details={'ignored': '\ud800'})]
    assert evaluate(candidate) == evaluate(baseline)
