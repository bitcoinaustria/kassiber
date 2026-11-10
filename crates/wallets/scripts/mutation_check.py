#!/usr/bin/env python3
"""Run R2's scratch-tree mutation checker for watch-only wallet services."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-wallets", "-p", "kassiber-daemon")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")

_run = checker._run
def _offline_run(command, *args, **kwargs):
    if command[:1] == ["cargo"]:
        command = [*command[:2], "--offline", *command[2:]]
    return _run(command, *args, **kwargs)
checker._run = _offline_run

if __name__ == "__main__":
    raise SystemExit(checker.main())
