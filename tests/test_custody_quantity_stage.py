"""Complete quantity stage parity from raw-only v1 snapshots."""
from collections import Counter
import json
from tests.custody_engine_compare import python_refusal
import pytest
from tests.custody_engine_compare import assert_same,permuted_snapshot,rust_results
from tests.custody_gap_stage import suites
from tests.custody_quantity_stage import python_stage
from tests.custody_quantity_composition_vectors import outcome


@pytest.mark.parametrize('suite,scenarios,generate',suites(),ids=lambda v:v if isinstance(v,str) else None)
def test_every_quantity_raw_generator_matches_both_engines(suite,scenarios,generate):
    counts={e:Counter() for e in ('Python','Rust')}
    from pathlib import Path
    inactive = set(json.loads(Path('tests/fixtures/custody_quantity_inactive_controls.json').read_text())[suite])
    # Preserve the inherited corpus and add one raw refusal to the same loop,
    # so the refusal-comparison branch is exercised rather than merely present.
    required = set(scenarios) - inactive
    if suite == 'evidence':
        required.add('raw_refusal')
    for seed in range(2000 + (suite == 'evidence')):
        if seed == 2000:
            book, case = raw_refusal_snapshot(), 'raw_refusal'
        else:
            book=generate(seed);case=scenarios[seed%len(scenarios)]
        for shuffled,b in enumerate((book,permuted_snapshot(book)) if seed<len(scenarios)*2 else (book,)):
            expected=outcome(lambda:python_stage(b))
            if 'error' in expected:
                import kassiber_tax
                actual=json.loads(kassiber_tax.custody(json.dumps(b)))
                assert actual['error']['code']==expected['error']
                assert actual['error']['stage'] == python_refusal(b, finalized=False)[1]
                if not shuffled:
                    for engine in counts: counts[engine][case] += 1
                continue
            stages,skipped=rust_results(b, finalized=False)
            assert 'quantity' in stages and skipped==('finalized_projection', 'fusion')
            assert_same(expected['expected'],stages['quantity'],f'{suite}:{case}:{seed}:{shuffled}')
            if not shuffled:
                for engine,state in (('Python',expected['expected']),('Rust',stages['quantity'])):
                    counts[engine][case]+=len(state['projection']['decisions'])+len(state['issues'])
    assert counts['Python']==counts['Rust']
    assert_activity(counts['Python'], required)
    assert not any(counts['Python'][case] for case in inactive)
    print(suite+' quantity active counts: '+json.dumps(counts,sort_keys=True))


def assert_activity(counts, required):
    assert required, "no active scenarios selected"
    assert all(counts[case] > 0 for case in required), "vacuous quantity scenario"


def test_vacuous_quantity_scenario_fails_activity_check():
    with pytest.raises(AssertionError, match="vacuous"):
        assert_activity(Counter({'vacuous': 0}), {'vacuous'})


def raw_refusal_snapshot():
    from pathlib import Path
    return next(c['input'] for c in json.loads(Path('tests/fixtures/custody_quantity_wrappers.json').read_text()) if c.get('error') == 'ValueError')


def test_raw_refusal_compares_both_engines():
    import kassiber_tax
    book = raw_refusal_snapshot()
    expected = outcome(lambda: python_stage(book))
    actual = json.loads(kassiber_tax.custody(json.dumps(book)))
    assert actual['error']['code'] == expected['error'] == 'ValueError'
    assert actual['error']['stage'] == python_refusal(book, finalized=False)[1]
