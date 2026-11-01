import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("lightning_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = (
    "-p", "kassiber", "-p", "kassiber-cli", "-p", "kassiber-daemon",
    "-p", "kassiber-chain", "-p", "kassiber-net", "--offline",
    "--features", "kassiber/test-harness,kassiber-chain/test-harness",
)
checker.MUTATIONS = Path(__file__).with_name("lightning_mutations.json")

if __name__ == "__main__":
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(checker.main())
