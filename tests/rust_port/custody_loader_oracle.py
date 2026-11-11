"""Opt-in local native-loader byte gate, including uncommitted caller writes.

The SQL copy stays on the child's pipe. It contains private book facts and must
never be included in a diagnostic or assertion. This test helper does not open
another connection to the source book, commit it, or copy only committed rows.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
from tests.rust_port.portable_dump import iterdump as _iterdump


ENV = "KASSIBER_TEST_CUSTODY_LOADER_BIN"


def exception_boundaries(error):
    """The non-debug code/message actually rendered by each product boundary.

    cli.main.main and daemon.run wrap unexpected loader/serializer exceptions
    in internal_error. The daemon additionally redacts the rendered message.
    Debug traceback/class details are deliberately outside this comparison.
    """
    from kassiber.daemon import _error_envelope
    from kassiber.envelope import build_error_envelope
    from kassiber.errors import AppError

    if isinstance(error, AppError):
        cli_code = error.code or "app_error"
        daemon_code = error.code or "app_error"
        message = str(error)
    else:
        cli_code = daemon_code = "internal_error"
        message = str(error) or type(error).__name__
    return {
        "cli": {key: value for key, value in build_error_envelope(cli_code, message)["error"].items()
                if key in ("code", "message")},
        "daemon": {key: value for key, value in _error_envelope(daemon_code, message)["error"].items()
                   if key in ("code", "message")},
    }


def compare(conn, profile, snapshot, *, binary=None):
    binary = binary or os.environ.get(ENV)
    if not binary:
        return
    if conn is None:
        # Pure stage tests can inject a snapshot without owning any database.
        return
    binary = Path(binary).resolve()
    if not binary.is_file():
        raise AssertionError("native custody loader probe is missing")
    request = {
        # sqlcipher3 does not expose Connection.iterdump; the pinned stdlib
        # dumper uses the same DB-API reads and includes pending writes.
        "sql": "\n".join(_iterdump(conn)),
        # JSON has no bytes; the probe reads a BLOB column as the byte tag.
        "profile": {key: {"$kassiber_bytes": bytes(value).hex()} if isinstance(value, (bytes, bytearray)) else value
                    for key, value in dict(profile).items()},
        "as_of": snapshot["as_of"],
    }
    result = subprocess.run(
        [str(binary)], input=json.dumps(request, ensure_ascii=True).encode("ascii"),
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120, check=False,
    )
    if result.returncode or result.stderr:
        raise AssertionError("native custody loader probe failed")
    try:
        actual = json.loads(result.stdout)
    except ValueError:
        raise AssertionError("native custody loader probe returned invalid output") from None
    try:
        expected = json.dumps(snapshot, ensure_ascii=True, allow_nan=True)
    except TypeError as error:
        # A snapshot holding a BLOB has no v1 text on either side. The probe
        # must refuse at that same encoding, as Python's own boundaries do.
        if actual.get("boundaries") != exception_boundaries(error):
            raise AssertionError("native custody loader v1 encoding differs for a snapshot with bytes") from None
        return actual
    if "error" in actual:
        raise AssertionError("native custody loader refused Python's snapshot: " + actual["error"])
    if actual["v1"] != expected:
        # Identify a byte offset, never the private accounting bytes.
        at = next((i for i, (a, b) in enumerate(zip(actual["v1"], expected)) if a != b),
                  min(len(actual["v1"]), len(expected)))
        raise AssertionError(f"native custody loader v1 differs at byte {at}")
    # Keep a count across Python children without retaining SQL or snapshot
    # bytes. The census session owns this disposable owner-only records file.
    records = os.environ.get("KASSIBER_TEST_CUSTODY_RECORDS")
    if records:
        fd = os.open(records + ".loader", os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            os.write(fd, b"equal\n")
        finally:
            os.close(fd)
    return actual
