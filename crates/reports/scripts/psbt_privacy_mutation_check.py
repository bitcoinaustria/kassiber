import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("psbt_privacy_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
CHECKER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CHECKER
SPEC.loader.exec_module(CHECKER)
CHECKER.ENGINE_PACKAGES = ("-p", "kassiber-reports", "--features", "test-harness", "--test", "psbt_privacy", "--offline")
CHECKER.MUTATIONS = Path(__file__).with_name("psbt_privacy_mutations.json")

if __name__ == "__main__":
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "strict"
    raise SystemExit(CHECKER.main())
