"""Run review assembly mutations with the existing disposable-tree checker."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-journals")
checker.MUTATIONS = Path(__file__).with_name("review_mutations.json")
_run = checker._run


def _offline_run(command, cwd, env):
    if command[0] == "cargo":
        command = [*command[:2], "--offline", "--features", "test-harness", *command[2:]]
    return _run(command, cwd, env)


checker._run = _offline_run
if target := os.environ.get("CARGO_TARGET_DIR"):
    os.environ["CARGO_BUILD_BUILD_DIR"] = target
os.environ["RUST_TEST_THREADS"] = "1"

if __name__ == "__main__":
    raise SystemExit(checker.main())
