#!/usr/bin/env python3
"""Run the scratch-tree mutation checker for main-sync slices S8 to S10.

Wallet completeness and `transactions verify` (S8), the transaction
dashboard's bucket size and years index (S9) and the readiness definitions
(S10): each entry mutates Rust source and names the differential pytest cases
or the crate unit tests (`<package>:<filter>` for `--lib`,
`<package>/<test binary>:<filter>` for an integration test) that must fail.

The scratch copy lives under the resolved ``target/``, and every scratch build
shares ``target/scratch-build`` so third-party crates compile once; delete it
when the campaign is done.

    uv run --locked python crates/daemon/scripts/sync_s8_s10_mutation_check.py [--only ID ...]
    uv run --locked python crates/daemon/scripts/sync_s8_s10_mutation_check.py --check
"""
import importlib.util
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.MUTATIONS = Path(__file__).with_name("sync_s8_s10_mutations.json")
checker.Mutation.runner = property(
    lambda self: "pytest" if all(test.startswith("tests/") for test in self.expect_failing) else "cargo"
)
BaseRunner = checker._Runner
CARGO_RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed")


class Runner(BaseRunner):
    def outcome(self, runner, test):
        if runner == "cargo":
            target, _, name = test.partition(":")
            package, _, binary = target.partition("/")
            selection = ["--test", binary] if binary else ["--lib"]
            result = checker._run(
                ["cargo", "test", "--offline", "--locked", "-p", package, *selection, "--", name],
                self.scratch, self.cargo_env,
            )
            selected = sum(int(p) + int(f) for p, f in CARGO_RESULT.findall(result.stdout))
            if result.returncode == 0:
                return ("passed" if selected else "error"), result.stdout[-2000:]
            return ("failed" if selected else "error"), (result.stdout + result.stderr)[-2000:]
        build = checker._run(
            ["cargo", "build", "--offline", "--locked", "-p", "kassiber", "--features", "test-harness"],
            self.scratch, self.cargo_env,
        )
        if build.returncode:
            return "error", build.stderr[-4000:]
        self.env["KASSIBER_RUST_BIN"] = str(Path(self.env["CARGO_TARGET_DIR"]) / "debug/kassiber")
        # Only the test process: cargo keeps the account's data home for its
        # toolchain, while tests must never resolve the real app-data root.
        self.env["XDG_DATA_HOME"] = str(self.scratch / "xdg-data")
        status, output = super().outcome("pytest", test)
        if re.search(r"\b\d+ skipped\b", output):
            return "error", output
        return status, output


def main():
    if "--check" in sys.argv[1:]:
        return checker.main()
    target = (ROOT / "target").resolve()
    scratch = target / "sync-s8-s10-mutation-scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ["TMPDIR"] = str(scratch)
    os.environ["CARGO_TARGET_DIR"] = str(target / "scratch-build" / "target")
    os.environ["CARGO_BUILD_BUILD_DIR"] = str(target / "scratch-build")
    os.environ.setdefault("CARGO_BUILD_JOBS", "8")
    os.environ.setdefault("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
    os.environ.setdefault("CARGO_PROFILE_TEST_DEBUG", "line-tables-only")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
