#!/usr/bin/env python3
"""Run journal mutations in the shared disposable-tree checker."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('tax_mutation_check', ROOT / 'tax-engine/scripts/mutation_check.py')
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ('-p', 'kassiber-journals')
checker.MUTATIONS = Path(__file__).with_name('mutations.json')

# Keep the shared runner offline, including its build-only mutation probes.
_run = checker._run

def _offline_run(command, cwd, env):
    if command[0] == 'cargo':
        command = [*command[:2], '--offline', *command[2:]]
        if '--' in command:
            command = [*command, '--test-threads=1']
        env = {**env, 'CARGO_BUILD_BUILD_DIR': str(cwd / 'target/build'), 'CARGO_TARGET_DIR': str(cwd / 'target'), 'CARGO_BUILD_JOBS': '8', 'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_INCREMENTAL': '0'}
    return _run(command, cwd, env)

checker._run = _offline_run

if __name__ == '__main__':
    raise SystemExit(checker.main())
