#!/usr/bin/env python3
"""Check journal CLI routing mutations with the shared disposable-tree runner."""
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('journal_mutations', Path(__file__).with_name('mutation_check.py'))
wrapper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wrapper)
checker = wrapper.checker
checker.ENGINE_PACKAGES = ('-p', 'kassiber-cli', '--features', 'test-probe')
checker.MUTATIONS = Path(__file__).with_name('route_mutations.json')

if __name__ == '__main__':
    raise SystemExit(checker.main())
