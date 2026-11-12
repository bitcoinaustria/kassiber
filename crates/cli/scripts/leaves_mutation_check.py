#!/usr/bin/env python3
"""Kill the final CLI leaves' mutations (journals, quarantine, identify,
analyze-file, reveal-descriptor, purge-legacy-ledger).

Entries naming cargo tests run over kassiber-cli, kassiber-journals and
kassiber-store; entries naming pytest node ids rebuild the harness binary in
the scratch tree and run `tests/test_rust_cli_leaves.py` against it. The scratch tree lives
under ``target/`` and is removed afterwards. Set ``CARGO_BUILD_BUILD_DIR`` to
one fixed directory per worktree (``$(realpath target)/scratch-build``) so
third-party crates compile once; every copied source is touched, so a previous
campaign's last mutant can never pass for this baseline.
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
checker.ENGINE_PACKAGES = ("-p", "kassiber-cli", "-p", "kassiber-journals", "-p", "kassiber-store")
checker.MUTATIONS = Path(__file__).with_name("leaves_mutations.json")
checker.Mutation.runner = property(
    lambda self: "pytest" if any("::" in test for test in self.expect_failing) else "cargo"
)

_run = checker._run
_copy = checker.copy_worktree


def _offline_run(command, *args, **kwargs):
    if command[:1] == ["cargo"]:
        command = [*command[:2], "--offline", *command[2:]]
    return _run(command, *args, **kwargs)


def _fresh_copy(root, scratch):
    _copy(root, scratch)
    for path in scratch.rglob("*.rs"):
        path.touch()


checker._run = _offline_run
checker.copy_worktree = _fresh_copy
BaseRunner = checker._Runner


class Runner(BaseRunner):
    def outcome(self, runner, test):
        if runner == "pytest":
            build = checker._run(
                ["cargo", "build", "--locked", "-p", "kassiber", "--features", "test-harness"],
                self.scratch, self.cargo_env,
            )
            if build.returncode:
                return "error", build.stderr[-4000:]
            self.env["KASSIBER_RUST_BIN"] = str(Path(self.cargo_env["CARGO_TARGET_DIR"]) / "debug/kassiber")
        return super().outcome(runner, test)


def main():
    target = (ROOT / "target").resolve()
    run = Path(tempfile.mkdtemp(prefix="leaves-mutation-", dir=target))
    scratch = run / "scratch"
    build = run / "build"
    scratch.mkdir()
    tempfile.tempdir = str(scratch)
    os.environ["TMPDIR"] = str(scratch)
    os.environ["CARGO_TARGET_DIR"] = str(build)
    os.environ.setdefault("CARGO_BUILD_BUILD_DIR", str(target / "scratch-build"))
    os.environ.setdefault("CARGO_BUILD_JOBS", "4")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(run, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
