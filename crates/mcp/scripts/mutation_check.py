#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the R6-8 MCP catalog.

The entries break the server's era selection, its JSON-RPC error codes, the
in-flight and subscription limits, request-id typing, cancellation and the
bounded drain; the result size measured on the escaped text; the access
gate's place and reason precedence; the brokered path's broker choice,
build and caller-context checks, re-redaction, stale-session replacement,
polling and cancel; the read-only book pin; #686's grants (the refusal
before resolution, the listing by grant and access mode, the capability a
call claims, the lease carrying and ending with the grant, the desktop lock
ending it, the writable pin's copy-back and the custody audit's `mcp_host`);
and `kassiber chat`'s
cooperative Ctrl-C: cancelling the turn, the second Ctrl-C, the prompt,
waiting for input or the daemon, the daemon's own process group and the
interrupt being handled once. Each must make its named Rust tests fail.

Set TMPDIR to a disposable directory outside /tmp; the scratch tree goes
there. CARGO_BUILD_BUILD_DIR, when set, is kept, so every scratch tree and
entry shares one build directory and third-party crates compile once.
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
    "-p", "kassiber-mcp", "-p", "kassiber-cli", "-p", "kassiber-setup",
    "-p", "kassiber-store", "-p", "kassiber-ipc", "-p", "kassiber-daemon",
    "-p", "kassiber-transfers",
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
    # The tests bind AF_UNIX sockets and pipes in disposable places only.
    os.environ.setdefault("KASSIBER_TEST_NO_EGRESS", "strict")
    raise SystemExit(checker.main())
