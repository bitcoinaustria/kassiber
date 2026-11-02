#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the R6-1 operator catalog.

The entries break the frame limits, the challenge comparison, the peer and
runtime-directory checks, the stale-socket unlink condition, build-identity
equality, connect timeouts, explicit inheritance and the owner janitor; each
must make its named Rust tests fail. Set TMPDIR to a disposable directory
outside /tmp; the scratch tree and its build go there.
"""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-operator", "-p", "kassiber-ipc", "-p", "kassiber-ownership")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")
_run = checker._run


def _offline(command, cwd, env):
    if command[0] == "cargo":
        command = [command[0], "--offline", *command[1:]]
    return _run(command, cwd, env)


checker._run = _offline

if __name__ == "__main__":
    # The tests bind AF_UNIX sockets in disposable directories only.
    os.environ.setdefault("KASSIBER_TEST_NO_EGRESS", "strict")
    raise SystemExit(checker.main())
