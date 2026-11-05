#!/usr/bin/env python3
"""Run the shared scratch-tree checker for main sync S7 (the Reports screen and
the crypto Kennzahlen from 1 March 2022), offline, over the two crates whose
rules it mutates."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("reports_screen_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-reports", "-p", "kassiber-journals", "--offline")
checker.MUTATIONS = Path(__file__).with_name("reports_screen_mutations.json")

if __name__ == "__main__":
    raise SystemExit(checker.main())
