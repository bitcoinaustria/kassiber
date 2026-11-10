import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("legacy_privacy_transport_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
CHECKER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CHECKER
SPEC.loader.exec_module(CHECKER)
CHECKER.ENGINE_PACKAGES = ("-p", "kassiber", "--features", "test-harness", "--test", "legacy_privacy_transport", "--offline")
CHECKER.MUTATIONS = Path(__file__).with_name("legacy_privacy_transport_mutations.json")

if __name__ == "__main__":
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "strict"
    raise SystemExit(CHECKER.main())
