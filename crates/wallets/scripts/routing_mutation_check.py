#!/usr/bin/env python3
"""Kill sync routing mutations through the registered CLI and hybrid launcher."""
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
checker.MUTATIONS = Path(__file__).with_name("routing_mutations.json")
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
        return super().outcome("pytest", test)


def main():
    scratch = ROOT / "target/sync-routing-mutation-scratch"
    build = ROOT / "target/sync-routing-mutation-build"
    scratch.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ["TMPDIR"] = str(scratch)
    os.environ["CARGO_TARGET_DIR"] = str(build)
    # A shared scratch build dir keeps third-party crates compiled across campaigns.
    os.environ.setdefault("CARGO_BUILD_BUILD_DIR", str(build))
    os.environ.setdefault("CARGO_BUILD_JOBS", "2")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(build, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
