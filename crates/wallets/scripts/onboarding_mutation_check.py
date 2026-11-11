#!/usr/bin/env python3
"""Kill desktop wallet-onboarding mutations.

Rust entries name cargo test filters over kassiber-wallets and kassiber-daemon;
entries naming pytest node ids rebuild the harness binary in the scratch tree
and run the daemon parity tests against it. Scratch and build trees live under
``target/`` and are removed afterwards.
"""
import dataclasses
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
checker.ENGINE_PACKAGES = ("-p", "kassiber-wallets", "-p", "kassiber-daemon")
checker.MUTATIONS = Path(__file__).with_name("onboarding_mutations.json")
checker.Mutation.runner = property(
    lambda self: "pytest" if any("::" in test for test in self.expect_failing) else "cargo"
)

_load = checker.load


def _pytest_or_cargo(path=checker.MUTATIONS):
    """An entry with pytest killers runs only those here; its Rust killers
    (kassiber black-box package filters) run under crates/mutation-check."""
    return [dataclasses.replace(m, expect_failing=tuple(t for t in m.expect_failing if ".py::" in t))
            if any(".py::" in t for t in m.expect_failing) else m for m in _load(path)]


checker.load = _pytest_or_cargo
_run = checker._run


def _offline_run(command, *args, **kwargs):
    if command[:1] == ["cargo"]:
        command = [*command[:2], "--offline", *command[2:]]
    return _run(command, *args, **kwargs)


checker._run = _offline_run
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
    # One private tree per run, so a concurrent `--check` cannot remove it.
    run = Path(tempfile.mkdtemp(prefix="onboarding-mutation-", dir=target))
    scratch = run / "scratch"
    build = run / "build"
    scratch.mkdir()
    tempfile.tempdir = str(scratch)
    os.environ["TMPDIR"] = str(scratch)
    os.environ["CARGO_TARGET_DIR"] = str(build)
    os.environ["CARGO_BUILD_BUILD_DIR"] = str(build)
    os.environ.setdefault("CARGO_BUILD_JOBS", "8")
    checker._Runner = Runner
    try:
        return checker.main()
    finally:
        shutil.rmtree(run, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
