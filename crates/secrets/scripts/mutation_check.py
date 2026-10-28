#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker for encryption boundaries."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-secrets", "-p", "kassiber-store")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")
_run = checker._run


def _offline(command, cwd, env):
    if command[0] == "cargo":
        command = [command[0], "--offline", *command[1:]]
    return _run(command, cwd, env)


checker._run = _offline

if __name__ == "__main__":
    raise SystemExit(checker.main())
