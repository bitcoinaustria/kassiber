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
Plain journal refusals may also replay in Python after the native service
proves byte-identical rollback; those calls include a `fallback_shape` in the log.

`daemon` runs in Python unless `KASSIBER_HYBRID_RUST_DAEMON=1`: one daemon
session mixes kinds, and most are not ported yet.

`KASSIBER_HYBRID_LOG=<file>` appends one JSON line per call,
`{"command": "wallets.list", "served_by": "rust"}`, so a slice can prove its
commands ran in Rust rather than falling back.

Leading `--hybrid-<name>=<value>` arguments set the same `KASSIBER_HYBRID_*`
settings from the candidate command line, for example
`["<python>", "<abs path>/hybrid_cli.py", "--hybrid-rust-bin=<bin>"]`. Tests
that scrub `KASSIBER_*` variables from a child's environment then still reach
the candidate. A variable present in the environment wins.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading

RUST_ENV = "KASSIBER_HYBRID_RUST_BIN"
LOG_ENV = "KASSIBER_HYBRID_LOG"
RUST_DAEMON_ENV = "KASSIBER_HYBRID_RUST_DAEMON"
IMPORT_ORACLE_ENV = "KASSIBER_HYBRID_IMPORT_ORACLE"
OPTION_PREFIX = "--hybrid-"
PROBE_NOT_IMPLEMENTED = 3  # crates/cli PROBE_NOT_IMPLEMENTED
SYNC_HANDOFF = 4
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
    if command == "wallets.sync" and (
        os.isatty(0) or any(arg.split("=", 1)[0] == "--db-passphrase-fd" for arg in args)
    ):
        return command, "python"
    return command, "rust"


def configure(argv: list[str]) -> list[str]:
    """Apply leading `--hybrid-*` settings and return the Kassiber arguments."""
    args = list(argv)
    while args and args[0].startswith(OPTION_PREFIX):
        name, separator, value = args.pop(0)[len(OPTION_PREFIX):].partition("=")
        if not separator:
            raise SystemExit(f"hybrid candidate: {OPTION_PREFIX}{name} needs =<value>")
        os.environ.setdefault("KASSIBER_HYBRID_" + name.upper().replace("-", "_"), value)
    return args


def python_command(args: list[str]) -> list[str]:
    owner_root = os.environ.get("KASSIBER_RUST_TEST_OWNER_ROOT")
    if owner_root:
        script = (
            "import sys; from pathlib import Path; from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )
        return [sys.executable, "-c", script, owner_root, *args]
    return [sys.executable, "-m", "kassiber", *args]


def journal_run(rust: str, args: list[str]):
    """Buffer native output until a proven whole-invocation replay decision.

    A passphrase pipe is one-shot. Retain its bounded bytes only in RAM and
    supply a fresh pipe to each child, including a possible Python replay.
    """
    fd_index = None
    inline = False
    secret = None
    for index, arg in enumerate(args):
        option, separator, value = arg.partition("=")
        if option.startswith("--") and "--db-passphrase-fd".startswith(option):
            inline = bool(separator)
            fd_index = index if inline else index + 1
    if fd_index is not None:
        try:
            raw = args[fd_index].partition('=')[2] if inline else args[fd_index]
            descriptor = int(raw)
            chunks = []
            remaining = 8193
            while remaining:
                chunk = os.read(descriptor, remaining)
                if not chunk:
                    break
                chunks.append(chunk)
                remaining -= len(chunk)
            secret = b"".join(chunks)
        except (ValueError, OverflowError, OSError, IndexError):
            fd_index = None

    def run(prefix, env):
        if fd_index is None:
            return subprocess.run([*prefix, *args], env=env, capture_output=True, check=False)
        read_fd, write_fd = os.pipe()
        child_args = list(args)
        child_args[fd_index] = ("--db-passphrase-fd=" if inline else "") + str(read_fd)
        def write():
            try:
                view = memoryview(secret)
                while view:
                    view = view[os.write(write_fd, view):]
            except BrokenPipeError:
                pass
            finally:
                os.close(write_fd)
        writer = threading.Thread(target=write)
        writer.start()
        try:
            return subprocess.run([*prefix, *child_args], env=env, pass_fds=(read_fd,), capture_output=True, check=False)
        finally:
            os.close(read_fd)
            writer.join()

    native = run([rust], {**os.environ, "KASSIBER_RUST_TEST_JOURNAL_FALLBACK": "1"})
    if native.returncode != PROBE_NOT_IMPLEMENTED:
        return native, "rust", None
    shape = native.stdout.decode("ascii").strip()
    # The reserved signal is emitted only by the guarded journal adapter.
    if native.stderr or not shape or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_:" for c in shape):
        raise SystemExit("hybrid candidate: invalid journal replay signal")
    reference = python_command([])
    return run(reference, os.environ.copy()), "python", shape


def main() -> None:
    args = configure(sys.argv[1:])
    rust = os.environ.get(RUST_ENV)
    if not rust:
        raise SystemExit(f"hybrid candidate: set {RUST_ENV} to a test-harness Rust build")
    command, served_by = route(rust, args)
    sync_result = None
    if command == "wallets.sync" and served_by == "rust":
        sync_result = subprocess.run(
            [rust, *args], capture_output=True, close_fds=False,
            env={**os.environ, "KASSIBER_RUST_TEST_SYNC_HANDOFF": "1"}, check=False,
        )
        if sync_result.returncode == SYNC_HANDOFF:
            if sync_result.stdout or sync_result.stderr:
                raise SystemExit("hybrid candidate: sync handoff must have empty output")
            served_by = "python"
            sync_result = None
    completed = None
    fallback_shape = None
    if served_by == "rust" and command == "journals.process":
        completed, served_by, fallback_shape = journal_run(rust, args)
    log = os.environ.get(LOG_ENV)
    if log:
        record = {"command": command, "served_by": served_by}
        if fallback_shape:
            record["fallback_shape"] = fallback_shape
        line = json.dumps(record) + "\n"
        fd = os.open(log, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        try:
            os.write(fd, line.encode("utf-8"))
        finally:
            os.close(fd)
    if completed is not None:
        sys.stdout.buffer.write(completed.stdout)
        sys.stderr.buffer.write(completed.stderr)
        raise SystemExit(completed.returncode)
    # exec keeps stdin, inherited descriptors (--db-passphrase-fd) and the
    # working directory exactly as the test passed them.
    if served_by == "rust":
        if sync_result is not None:
            sys.stdout.buffer.write(sync_result.stdout)
            sys.stderr.buffer.write(sync_result.stderr)
            raise SystemExit(sync_result.returncode)
        if command in {"wallets.import-wasabi", "wallets.import-btcpay", "wallets.import-csv", "wallets.import-phoenix", "wallets.import-json", "wallets.import-river", "wallets.import-bull", "wallets.import-bullbitcoin", "wallets.import-coinfinity", "wallets.import-21bitcoin", "wallets.import-pocket", "wallets.import-pocketbitcoin", "wallets.import-strike", "wallets.import-ledger-live", "wallets.import-cointracking", "wallets.import-blockpit", "wallets.import-binance-supplemental", "wallets.import-ledger", "wallets.import-samourai"} and os.environ.get(IMPORT_ORACLE_ENV) == "1":
            # Direct script execution puts tests/rust_port first on sys.path.
            sys.path.insert(0, str(__import__("pathlib").Path(__file__).resolve().parents[2]))
            from tests.rust_port.file_import_oracle import run
            result = run(rust, args)
            sys.stdout.buffer.write(result.stdout)
            sys.stderr.buffer.write(result.stderr)
            raise SystemExit(result.returncode)
        if command == "daemon":
            # Only this development launcher enables per-job reference replay.
            # Production builds contain no Python journal bridge.
            from pathlib import Path
            os.environ["KASSIBER_RUST_TEST_JOURNAL_PYTHON"] = sys.executable
            os.environ["KASSIBER_RUST_TEST_JOURNAL_REFERENCE"] = str(
                Path(__file__).with_name("journal_job_reference.py").resolve()
            )
        os.execv(rust, [rust, *args])
    # Match the native test build's disposable ownership namespace. Python's
    # production namespace deliberately uses the UID account home, not HOME.
    # Keep this injection in the test launcher rather than product code.
    os.execv(sys.executable, python_command(args))


if __name__ == "__main__":
    main()
