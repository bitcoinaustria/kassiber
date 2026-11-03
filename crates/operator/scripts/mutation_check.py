#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the R6 operator catalog.

The R6-1 entries break the frame limits, the challenge comparison, the peer
and runtime-directory checks, the stale-socket unlink condition,
build-identity equality, connect timeouts, explicit inheritance and the owner
janitor. The R6-2 entries break the broker service's admission order,
capability and scope checks, replay fingerprint, retention, expiry, admin
authorization, result states, lease-ending commands and argv pinning, the
server's limits, detail filter and build check, the login guard, the backoff
threshold and the runner's caller context. The R6-3 entries break the
agent-access file's parsing and reason precedence, the agent-session pid,
token, denial memory, scope and heartbeat checks, the desktop lease's check
and retry intervals and conditional lock, and the brokered CLI's broker
choice, Ctrl-C exit, argument pinning and result mapping. Each must make its
named Rust tests fail. Set TMPDIR to a disposable directory
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
checker.ENGINE_PACKAGES = (
    "-p", "kassiber-operator", "-p", "kassiber-ipc", "-p", "kassiber-ownership",
    "-p", "kassiber-setup", "-p", "kassiber-daemon", "-p", "kassiber-cli",
    # The CLI's in-process run tests need its probe.
    "--features", "kassiber-cli/test-probe",
)
# The first build of a scratch tree compiles these packages from nothing.
checker.TEST_TIMEOUT_SECONDS = 3 * 60 * 60
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
