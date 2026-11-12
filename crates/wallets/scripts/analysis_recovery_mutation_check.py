import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "mirror_recovery_mutation_runner", ROOT / "tax-engine/scripts/mutation_check.py"
)
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)
checker.MUTATIONS = Path(__file__).with_name("analysis_recovery_mutations.json")


class NativeRunner(checker._Runner):
    def __init__(self, scratch, env):
        super().__init__(scratch, env)
        self.cargo_env["CARGO_BUILD_BUILD_DIR"] = str(scratch / "target/build")

    def builds(self):
        result = checker._run(
            [
                "cargo", "build", "--offline", "--locked", "--manifest-path",
                str(self.manifest), "-p", "kassiber-wallets", "--features",
                "test-harness", "--example", "analysis_durable_fixture",
                "--target-dir", str(self.scratch / "target"),
            ],
            self.scratch,
            self.cargo_env,
        )
        self.env["KASSIBER_ANALYSIS_DURABLE_BIN"] = str(
            self.scratch / "target/debug/examples/analysis_durable_fixture"
        )
        self.env["KASSIBER_TEST_NO_EGRESS"] = "strict"
        return result.returncode == 0, result.stderr[-2000:]

    def outcome(self, runner, test):
        ok, output = self.builds()
        if not ok:
            return "error", output
        checker._check_imports_resolve_to(self.scratch, self.env)
        result = checker._run(
            [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", test],
            self.scratch,
            self.env,
        )
        status = {0: "passed", 1: "failed"}.get(result.returncode, "error")
        if "skipped" in result.stdout:
            status = "error"
        return status, (result.stdout + result.stderr)[-5000:]


checker._Runner = NativeRunner

if __name__ == "__main__":
    raise SystemExit(checker.main())
