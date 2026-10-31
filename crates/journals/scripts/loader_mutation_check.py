#!/usr/bin/env python3
"""Run the experimental loader's branch mutation catalog in a disposable tree."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-journals", "-p", "kassiber-custody",
                           "-p", "kassiber-pyjson", "-p", "kassiber-pytext", "-p", "kassiber-envelope")
checker.MUTATIONS = Path(__file__).with_name("loader_mutations.json")

# A machine-wide build-dir template would create a separate intermediate tree
# for the disposable checkout. Keep all mutation output in the requested tree.
if target := os.environ.get("CARGO_TARGET_DIR"):
    os.environ.setdefault("CARGO_BUILD_BUILD_DIR", target)

if __name__ == "__main__":
    raise SystemExit(checker.main())
