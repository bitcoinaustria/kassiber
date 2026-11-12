#!/usr/bin/env python3
"""Check journal CLI routing mutations with the shared disposable-tree runner.

Entries on crates/journals (JROUTE-auto-pair-service) name a journals
integration test; check them with mutation_check.py --mutations route_mutations.json --only ID.
"""
import importlib.util
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('journal_mutations', Path(__file__).with_name('mutation_check.py'))
wrapper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wrapper)
checker = wrapper.checker
checker.ENGINE_PACKAGES = ('-p', 'kassiber-cli', '--features', 'test-probe')
checker.MUTATIONS = Path(__file__).with_name('route_mutations.json')


def _cli_test_target(test):
    """The kassiber-cli integration target that defines `test`, else its lib."""
    name = test.rsplit('::', 1)[-1]
    pattern = re.compile(r'\bfn\s+' + re.escape(name) + r'\s*\(')
    owners = [path.stem for path in (ROOT / 'crates/cli/tests').glob('*.rs') if pattern.search(path.read_text())]
    if len(owners) > 1:
        raise ValueError(f'ambiguous integration test filter: {test}')
    return ['--test', owners[0]] if owners else ['--lib']


# The journal wrapper looks for owners under crates/journals/tests.
wrapper._test_target = _cli_test_target

if __name__ == '__main__':
    raise SystemExit(checker.main())
