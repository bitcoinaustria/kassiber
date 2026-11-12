"""Run raw review composition and authoring mutations in an offline copy."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("--offline", "-p", "kassiber-transfers", "-p", "kassiber-journals", "-p", "kassiber-transactions",
                           "--features", "kassiber-transfers/test-harness,kassiber-journals/test-harness")
checker.MUTATIONS = Path(__file__).with_name("review_closure_mutations.json")
os.environ["RUST_TEST_THREADS"] = "1"

if __name__ == "__main__":
    raise SystemExit(checker.main())
