#!/usr/bin/env python3
"""Run the shared scratch-tree checker for native report rules, offline."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("report_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-reports", "-p", "kassiber-pytext", "-p", "kassiber-daemon", "-p", "kassiber-cli", "-p", "kassiber-store", "--offline")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")

if __name__ == "__main__":
    raise SystemExit(checker.main())
