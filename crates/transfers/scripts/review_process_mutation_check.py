#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the main-sync S4 rules that
only the Python differential tests observe: review cases and badges, the cached
transfer-candidate recounts, ui.journals.process's auto_pair and #711's book
currency on a quarantine resolve.

Each entry mutates Rust source; its tests are pytest node ids run against a
`kassiber` binary rebuilt (with `test-harness`) from the mutated scratch tree.
Scratch trees and build outputs live under the worktree's `target/`; builds
share `target/scratch-build` (override with CARGO_BUILD_BUILD_DIR).
"""
import importlib.util
import os
from pathlib import Path
import shutil
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.MUTATIONS = Path(__file__).with_name("review_process_mutations.json")
checker.Mutation.runner = property(lambda self: "pytest")
BaseRunner = checker._Runner


class Runner(BaseRunner):
    def outcome(self, runner, test):
        build = checker._run(
            ["cargo", "build", "--offline", "--locked", "-p", "kassiber", "--features", "test-harness"],
            self.scratch, self.cargo_env,
        )
        if build.returncode:
            return "error", build.stderr[-4000:]
        self.env["KASSIBER_RUST_BIN"] = str(Path(self.env["CARGO_TARGET_DIR"]) / "debug/kassiber")
        # The journal desktop fixtures require data roots under the tree's target/.
        (self.scratch / "target").mkdir(exist_ok=True)
        self.env["PYTEST_ADDOPTS"] = f"--basetemp={self.scratch / 'target/review-mutation-tests'}"
        return super().outcome("pytest", test)


def main():
    if "--check" in sys.argv[1:]:
        return checker.main()
    target = (ROOT / "target").resolve()
    scratch = target / "review-mutation-scratch"
    build = Path(os.environ.get("CARGO_BUILD_BUILD_DIR", target / "scratch-build"))
    scratch.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ["TMPDIR"] = str(scratch)
    os.environ["CARGO_TARGET_DIR"] = str(build)
    os.environ["CARGO_BUILD_BUILD_DIR"] = str(build)
    os.environ.setdefault("CARGO_BUILD_JOBS", "16")
    os.environ.setdefault("CARGO_PROFILE_DEV_DEBUG", "line-tables-only")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
