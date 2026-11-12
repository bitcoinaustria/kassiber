#!/usr/bin/env python3
"""Scratch-tree mutation checks for exchange API sync and its shared seams.

Uses the shared checker: named baselines must pass, mutations must build and
every named selection must fail. Scratch trees and the one shared build dir
live under this checkout's target/.
"""
import importlib.util
import os
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-wallets", "--lib", "--features", "test-harness", "--offline")
checker.MUTATIONS = Path(__file__).with_name("exchange_mutations.json")

if __name__ == "__main__":
    target = (ROOT / "target").resolve()
    scratch = target / "exchange-mutation-scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ.setdefault("CARGO_BUILD_BUILD_DIR", str(target / "scratch-build"))
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
