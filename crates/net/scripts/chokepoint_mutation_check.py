#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the network chokepoint catalog.

    uv run --locked python crates/net/scripts/chokepoint_mutation_check.py [--only ID ...] [--check]

Set TMPDIR to a disposable directory for the scratch tree, and
CARGO_BUILD_BUILD_DIR to one fixed directory reused across runs.
"""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-net", "--test", "chokepoint", "--offline")
checker.MUTATIONS = Path(__file__).with_name("chokepoint_mutations.json")

if __name__ == "__main__":
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
