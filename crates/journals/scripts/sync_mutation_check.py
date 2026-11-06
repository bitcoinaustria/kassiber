#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the main-sync catalog for
the transaction graph's full asset ids (S2).

    uv run --locked python crates/journals/scripts/sync_mutation_check.py [--only ID ...] [--check]
"""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("--offline", "-p", "kassiber-journals", "--lib")
checker.MUTATIONS = Path(__file__).with_name("sync_mutations.json")

if __name__ == "__main__":
    raise SystemExit(checker.main())
