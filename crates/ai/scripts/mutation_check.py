#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker for the AI tool executor (R6-5) and
providers (R6-6a, R6-6b).

Entries whose id starts with ``wire-`` are killed by pytest process oracles
in ``tests/test_rust_ai_tools.py``: the scratch tree's test-harness binary is
rebuilt before each such test and named by ``KASSIBER_RUST_BIN``. Other
entries run cargo test filters over ``kassiber-ai`` and ``kassiber-transfers``.
Scratch trees live under ``target/`` and are removed afterwards; every build
shares ``target/scratch-build``, so third-party crates compile once. A skipped
process test counts as an error, never a kill.

    uv run --locked python crates/ai/scripts/mutation_check.py [--only ID ...]
    uv run --locked python crates/ai/scripts/mutation_check.py --check
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
checker.ENGINE_PACKAGES = ("--offline", "-p", "kassiber-ai", "-p", "kassiber-net", "-p", "kassiber-transfers")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")
checker.Mutation.runner = property(lambda m: "pytest" if m.id.startswith("wire-") else "cargo")
BaseRunner = checker._Runner


class Runner(BaseRunner):
    def outcome(self, runner, test):
        if runner == "pytest":
            build = checker._run(
                ["cargo", "build", "--offline", "--locked", "-p", "kassiber", "--features", "test-harness"],
                self.scratch, self.cargo_env)
            if build.returncode:
                return "error", build.stderr[-3000:]
            self.env["KASSIBER_RUST_BIN"] = str(Path(self.env["CARGO_TARGET_DIR"]) / "debug/kassiber")
        status, output = super().outcome(runner, test)
        if re.search(r"\b[1-9]\d* skipped\b", output):
            return "error", output
        return status, output


def main():
    if "--check" in sys.argv[1:]:
        return checker.main()
    # Disposable data roots must not contain a symlink; resolve target/.
    target = (ROOT / "target").resolve()
    scratch = target / "ai-mutation-scratch"
    build = target / "scratch-build"
    scratch.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ.update(
        TMPDIR=str(scratch), CARGO_TARGET_DIR=str(build), CARGO_BUILD_BUILD_DIR=str(build),
        CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0")
    os.environ.setdefault("CARGO_BUILD_JOBS", "8")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
