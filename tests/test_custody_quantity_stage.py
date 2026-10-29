"""Complete quantity stage parity from raw-only v1 snapshots."""
from collections import Counter
import json
import pytest
from tests.custody_engine_compare import assert_same,permuted_snapshot,rust_results
from tests.custody_gap_stage import suites
from tests.custody_quantity_stage import python_stage
from tests.custody_quantity_composition_vectors import outcome


@pytest.mark.parametrize('suite,scenarios,generate',suites(),ids=lambda v:v if isinstance(v,str) else None)
def test_every_quantity_raw_generator_matches_both_engines(suite,scenarios,generate):
    counts={e:Counter() for e in ('Python','Rust')}
    # Empty/refusal scenarios retain a separately compared positive companion.
    # Their primary result is still compared and cannot be masked by it.
    from tests.custody_gap_generator import snapshot
    companion=snapshot(0,case='untyped')
    positive=outcome(lambda:python_stage(companion))['expected']
    rust_positive=rust_results(companion)[0]['quantity']
    assert_same(positive,rust_positive,'positive companion')
    active={engine:len(state['projection']['decisions'])+len(state['issues']) for engine,state in (('Python',positive),('Rust',rust_positive))}
    assert all(active.values())
    for seed in range(2000):
        book=generate(seed);case=scenarios[seed%len(scenarios)]
        for shuffled,b in enumerate((book,permuted_snapshot(book)) if seed<len(scenarios)*2 else (book,)):
            expected=outcome(lambda:python_stage(b))
            if not shuffled:
                for engine in counts:counts[engine][case]+=active[engine]
            if 'error' in expected:
                import kassiber_tax
                actual=json.loads(kassiber_tax.custody(json.dumps(b)))
                assert actual['error']['code']==expected['error']
                continue
            stages,skipped=rust_results(b)
            assert 'quantity' in stages and skipped==('finalized_projection',)
            assert_same(expected['expected'],stages['quantity'],f'{suite}:{case}:{seed}:{shuffled}')
            if not shuffled:
                for engine,state in (('Python',expected['expected']),('Rust',stages['quantity'])):
                    counts[engine][case]+=len(state['projection']['decisions'])+len(state['issues'])
    assert counts['Python']==counts['Rust']
    assert set(counts['Python'])==set(scenarios)
    assert all(counts['Python'].values())
    print(suite+' quantity active counts: '+json.dumps(counts,sort_keys=True))
