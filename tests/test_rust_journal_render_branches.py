import json

from tests.rust_port.fixture_compare import assert_rebuilt
from tests.rust_port.journal_render_branches import PATH, renderer_vectors, vectors
from tests.test_tax_mutations import checker


def test_renderer_branch_witnesses_rebuild_from_python():
    witnesses = json.loads(PATH.read_text())
    assert_rebuilt(witnesses, vectors() + renderer_vectors(), PATH.name)
    assert len({case['name'] for case in witnesses}) == len(witnesses)
    for case in witnesses:
        assert set(case['expected']) in ({'state'}, {'error'})
        assert case['entry_ids'] == [item['python'] for item in case['uuid_mapping']]
        if 'state' in case['expected']:
            state = json.loads(case['expected']['state'])
            assert [entry['id'] for entry in state['entries']] == case['entry_ids']


def test_renderer_branch_mutations_apply_and_name_existing_tests():
    root = PATH.parents[2]
    mutations = (checker.load(root / 'scripts/branch_mutations.json')
                 + checker.load(root / 'scripts/builder_mutations.json'))
    assert mutations
    assert checker.problems(mutations) == []
    names = {mutation.id for mutation in checker.load(root / 'scripts/mutations.json')}
    assert not names.intersection(mutation.id for mutation in mutations)
    tests = '\n'.join(path.read_text() for path in (root / 'tests').glob('*.rs'))
    for mutation in mutations:
        for test in mutation.expect_failing:
            assert f'fn {test}(' in tests, (mutation.id, test)
