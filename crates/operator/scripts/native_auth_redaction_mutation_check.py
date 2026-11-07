#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the Touch ID helper-error catalog.

    uv run --locked python crates/operator/scripts/native_auth_redaction_mutation_check.py [--only ID ...] [--check]

The catalog covers the allowlist that lets the helper's fixed error texts
through `sanitize_traceback_text` in both runtimes while anything else after
`auth:` stays redacted. Set TMPDIR to a disposable directory for the scratch
tree, and CARGO_BUILD_BUILD_DIR to one fixed directory reused across runs.
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
checker.ENGINE_PACKAGES = ("-p", "kassiber-envelope", "-p", "kassiber-operator", "--offline")
checker.MUTATIONS = Path(__file__).with_name("native_auth_redaction_mutations.json")

if __name__ == "__main__":
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
