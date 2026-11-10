#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the envelope's secret floor."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-envelope", "--offline")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")

if __name__ == "__main__":
    # The scanner tests are pure functions over in-memory strings.
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
