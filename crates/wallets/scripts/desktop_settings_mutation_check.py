#!/usr/bin/env python3
"""Run the scratch-tree mutation checker for the desktop settings kinds.

Each entry mutates Rust source and names the differential pytest cases in
``tests/test_rust_desktop_settings.py`` (and its siblings) that must fail.
The scratch copy, its Cargo target and every disposable book live under the
resolved ``target/`` directory, never under the system temporary directory.

    uv run --locked python crates/wallets/scripts/desktop_settings_mutation_check.py [--only ID ...]
    uv run --locked python crates/wallets/scripts/desktop_settings_mutation_check.py --check
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
checker.MUTATIONS = Path(__file__).with_name("desktop_settings_mutations.json")
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
    scratch = target / "desktop-settings-mutation-scratch"
    build = target / "desktop-settings-mutation-build"
    scratch.mkdir(parents=True, exist_ok=True)
    tempfile.tempdir = str(scratch)
    os.environ["TMPDIR"] = str(scratch)
    os.environ["CARGO_TARGET_DIR"] = str(build)
    os.environ["CARGO_BUILD_BUILD_DIR"] = str(build)
    os.environ.setdefault("CARGO_BUILD_JOBS", "8")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(build, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
