#!/usr/bin/env python3
"""Run journal mutations in the shared disposable-tree checker."""
import importlib.util
import os
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('tax_mutation_check', ROOT / 'tax-engine/scripts/mutation_check.py')
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ('-p', 'kassiber-journals', '--features', 'test-harness')
checker.MUTATIONS = Path(__file__).with_name('mutations.json')
if target := os.environ.get('CARGO_TARGET_DIR'):
    os.environ.setdefault('CARGO_BUILD_BUILD_DIR', target)

# Keep the shared runner offline, including its build-only mutation probes.
_run = checker._run


def _test_target(test):
    """Select the owning target without rebuilding unrelated examples/tests."""
    name = test.rsplit('::', 1)[-1]
    pattern = re.compile(r'\bfn\s+' + re.escape(name) + r'\s*\(')
    owners = [path.stem for path in (ROOT / 'crates/journals/tests').glob('*.rs')
              if pattern.search(path.read_text())]
    if pattern.search((ROOT / 'crates/journals/examples/journal-persist.rs').read_text()):
        owners.append('probe')
    if len(owners) > 1:
        raise ValueError(f'ambiguous integration test filter: {test}')
    return ['--test', owners[0]] if owners else ['--lib']


def _offline_run(command, cwd, env):
    if command[0] == 'cargo':
        command = [*command[:2], '--offline', *command[2:]]
        if '--' in command:
            split = command.index('--')
            command = [*command[:split], *_test_target(command[split + 1]), *command[split:]]
            command = [*command, '--test-threads=1']
        elif '--no-run' in command:
            # Every mutant must compile. The selected integration target also
            # compiles in outcome(); build errors there remain checker errors.
            command = [*command, '--lib']
        env = {**env, 'CARGO_BUILD_BUILD_DIR': str(cwd / 'target/build'), 'CARGO_TARGET_DIR': str(cwd / 'target'), 'CARGO_BUILD_JOBS': '8', 'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_INCREMENTAL': '0'}
    return _run(command, cwd, env)

checker._run = _offline_run

if __name__ == '__main__':
    raise SystemExit(checker.main())
