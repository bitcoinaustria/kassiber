#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the chain protocol catalog."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-chain", "-p", "kassiber-net", "--features", "kassiber-chain/test-harness", "--offline")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")

if __name__ == "__main__":
    # Independent attempt observation is active for every named mutant test;
    # external destinations remain blocked even if a regression bypasses policy.
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
