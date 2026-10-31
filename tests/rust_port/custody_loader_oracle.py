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
from sqlite3.dump import _iterdump


ENV = "KASSIBER_TEST_CUSTODY_LOADER_BIN"


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
        "profile": dict(profile),
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
    if "error" in actual:
        raise AssertionError("native custody loader refused Python's snapshot: " + actual["error"])
    expected = json.dumps(snapshot, ensure_ascii=True, allow_nan=True)
    if actual["v1"] != expected:
        # Identify a byte offset, never the private accounting bytes.
        at = next((i for i, (a, b) in enumerate(zip(actual["v1"], expected)) if a != b),
                  min(len(actual["v1"]), len(expected)))
        raise AssertionError(f"native custody loader v1 differs at byte {at}")
    return actual
