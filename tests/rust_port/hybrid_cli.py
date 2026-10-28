#!/usr/bin/env python3
"""Test-only CLI candidate that serves each call from Rust when it can.

Point `KASSIBER_TEST_CLI` at `["<python>", "tests/rust_port/hybrid_cli.py"]`
and `KASSIBER_HYBRID_RUST_BIN` at a `--features test-harness` build of
`crates/kassiber`. For each call the candidate first asks that binary, with
`KASSIBER_RUST_TEST_PROBE=1`, whether it implements the command; the probe
parses the command line and touches nothing. Implemented commands and usage
errors then run in Rust, everything else in this checkout's Python. Both
share books safely (plan 21, R3 coexistence), so a test that sets up its book
through commands Rust lacks still exercises the commands it has.

`daemon` runs in Python unless `KASSIBER_HYBRID_RUST_DAEMON=1`: one daemon
session mixes kinds, and most are not ported yet.

`KASSIBER_HYBRID_LOG=<file>` appends one JSON line per call,
`{"command": "wallets.list", "served_by": "rust"}`, so a slice can prove its
commands ran in Rust rather than falling back.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys

RUST_ENV = "KASSIBER_HYBRID_RUST_BIN"
LOG_ENV = "KASSIBER_HYBRID_LOG"
RUST_DAEMON_ENV = "KASSIBER_HYBRID_RUST_DAEMON"
PROBE_NOT_IMPLEMENTED = 3  # crates/cli PROBE_NOT_IMPLEMENTED
USAGE_ERROR = 2


def route(rust: str, args: list[str]) -> tuple[str, str]:
    """The command path the probe resolved and which side serves it."""
    probe = subprocess.run(
        [rust, *args],
        env={**os.environ, "KASSIBER_RUST_TEST_PROBE": "1"},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    if probe.returncode == USAGE_ERROR:
        return "<usage>", "rust"
    if probe.returncode not in (0, PROBE_NOT_IMPLEMENTED):
        raise SystemExit(
            f"hybrid candidate: {rust} probe exited {probe.returncode}; "
            "build it with --features test-harness"
        )
    command = probe.stdout.decode("utf-8").strip()
    if probe.returncode == PROBE_NOT_IMPLEMENTED:
        return command, "python"
    if command == "daemon" and os.environ.get(RUST_DAEMON_ENV) != "1":
        return command, "python"
    return command, "rust"


def main() -> None:
    rust = os.environ.get(RUST_ENV)
    if not rust:
        raise SystemExit(f"hybrid candidate: set {RUST_ENV} to a test-harness Rust build")
    args = sys.argv[1:]
    command, served_by = route(rust, args)
    log = os.environ.get(LOG_ENV)
    if log:
        line = json.dumps({"command": command, "served_by": served_by}) + "\n"
        fd = os.open(log, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(fd, line.encode("utf-8"))
        finally:
            os.close(fd)
    # exec keeps stdin, inherited descriptors (--db-passphrase-fd) and the
    # working directory exactly as the test passed them.
    if served_by == "rust":
        os.execv(rust, [rust, *args])
    # Match the native test build's disposable ownership namespace. Python's
    # production namespace deliberately uses the UID account home, not HOME.
    # Keep this injection in the test launcher rather than product code.
    owner_root = os.environ.get("KASSIBER_RUST_TEST_OWNER_ROOT")
    if owner_root:
        script = (
            "import sys; from pathlib import Path; from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )
        os.execv(sys.executable, [sys.executable, "-c", script, owner_root, *args])
    os.execv(sys.executable, [sys.executable, "-m", "kassiber", *args])


if __name__ == "__main__":
    main()
