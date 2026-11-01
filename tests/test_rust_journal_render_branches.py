import json

from tests.rust_port.journal_render_branches import PATH, vectors
from tests.test_tax_mutations import checker


def test_renderer_branch_witnesses_rebuild_from_python():
    assert json.loads(PATH.read_text()) == vectors()


def test_renderer_branch_mutations_apply_and_name_existing_tests():
    root = PATH.parents[2]
    mutations = checker.load(root / 'scripts/branch_mutations.json')
    assert mutations
    assert checker.problems(mutations) == []
    names = {mutation.id for mutation in checker.load(root / 'scripts/mutations.json')}
    assert not names.intersection(mutation.id for mutation in mutations)
    tests = '\n'.join(path.read_text() for path in (root / 'tests').glob('*.rs'))
    for mutation in mutations:
        for test in mutation.expect_failing:
            assert f'fn {test}(' in tests, (mutation.id, test)
