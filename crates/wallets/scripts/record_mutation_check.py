#!/usr/bin/env python3
"""Scratch-tree mutations for Bitcoin normalization and observer composition."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-wallets", "--lib", "--features", "test-harness", "--offline")
checker.MUTATIONS = Path(__file__).with_name("record_mutations.json")


class IsolatedRunner(checker._Runner):
    def __init__(self, scratch, env):
        super().__init__(scratch, env)
        self.cargo_env["CARGO_BUILD_BUILD_DIR"] = str(scratch / "target/build")
        self.cargo_env["CARGO_TARGET_DIR"] = str(scratch / "target")


checker._Runner = IsolatedRunner

if __name__ == "__main__":
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
