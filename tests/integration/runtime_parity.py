"""Compare what the Python and Rust runtimes leave in a book (test code only).

Both runtimes run the same steps from the same starting state at the same
path. Envelopes and every table of the store's schema manifest must then
match once fresh identities and clocks are masked.
"""
from __future__ import annotations

from datetime import datetime
from functools import lru_cache
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess

ROOT = Path(__file__).resolve().parents[2]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
# Generated identities: dashed UUIDs and the bare 32-hex form (database instance ids).
UUID = re.compile(r"\b(?:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{32})\b")
CLOCK = re.compile(r"(\d{4}-\d{2}-\d{2})[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?")
HEX64 = re.compile(r"\b[0-9a-f]{64}\b")
# Wall-clock durations; the service oracles normalize them too (rust-shell.md).
WALL_CLOCK_KEYS = frozenset({"elapsed_ms"})


@lru_cache(maxsize=None)
def rust_guard_capable(binary: str) -> bool:
    """Whether ``binary`` carries the test socket guard (a test-harness build).

    A release build refuses an armed ``KASSIBER_TEST_NO_EGRESS`` with exit 78.
    """
    env = {key: value for key, value in os.environ.items() if not key.startswith("KASSIBER_")}
    env["KASSIBER_TEST_NO_EGRESS"] = "1"
    result = subprocess.run([binary, "--version"], env=env, capture_output=True, timeout=60)
    return result.returncode != 78


def child_env(home: Path, *, python_child: bool, rust_bin: str | None = None) -> dict[str, str]:
    """A disposable environment for one runtime's children.

    Preferences, XDG roots and HOME point under ``home``, so neither runtime
    reads the account's offline switch or app-data root. The product kill
    switch is off because it refuses loopback for Sync; the test socket guard
    blocks every non-loopback destination where the child can carry it: a
    Python child through ``tests/_child_hooks``, a Rust child only when it is
    a test-harness build.
    """
    from tests.test_rust_shell_parity import _environment

    env = _environment(home)
    env.pop("KASSIBER_NO_EGRESS", None)
    env.pop("KASSIBER_TEST_CLI", None)
    env["KASSIBER_STATE_ROOT"] = str(home / "state")
    env["TZ"] = "UTC"
    guarded = python_child or (rust_bin is not None and rust_guard_capable(rust_bin))
    if python_child:
        env["PYTHONPATH"] = os.pathsep.join([str(ROOT / "tests" / "_child_hooks"), str(ROOT)])
    if guarded:
        env["KASSIBER_TEST_NO_EGRESS"] = "1"
    else:
        env.pop("KASSIBER_TEST_NO_EGRESS", None)
    if python_child:
        from tests.egress_guard import require_guarded_child

        require_guarded_child(env)
    return env


class Masker:
    """Fresh identities and clocks become stable placeholders.

    Identities in ``known`` (the shared starting book) stay literal. New ones
    are numbered in order of first appearance while reading envelopes, whose
    order both runtimes share; after ``freeze()`` any further new identity,
    such as a row key only the book holds, becomes ``<id>``. Clocks within two
    days of ``today`` and recorded artifact digests are masked too.
    """

    def __init__(self, known: set[str], today: datetime, *, anonymous: bool = False):
        self.known = known
        self.today = today.date()
        self.fresh: dict[str, str] = {}
        self.digests: set[str] = set()
        self.frozen = False
        # Every fresh identity is `<id>` and every unknown 64-hex value `<hex>`.
        self.anonymous = anonymous

    def freeze(self) -> None:
        self.frozen = True

    def text(self, value: str) -> str:
        def identity(match):
            token = match.group(0)
            if token in self.known:
                return token
            if self.anonymous:
                return "<id>"
            if token in self.fresh or not self.frozen:
                return self.fresh.setdefault(token, f"<id-{len(self.fresh)}>")
            return "<id>"

        def clock(match):
            day = datetime.strptime(match.group(1), "%Y-%m-%d").date()
            return "<clock>" if abs((day - self.today).days) <= 2 else match.group(0)

        def digest(match):
            token = match.group(0)
            if token in self.digests:
                return "<artifact-sha256>"
            return "<hex>" if self.anonymous and token not in self.known else token

        value = UUID.sub(identity, value)
        value = CLOCK.sub(clock, value)
        return HEX64.sub(digest, value)

    def value(self, value):
        if isinstance(value, str):
            return self.text(value)
        if isinstance(value, list):
            return [self.value(v) for v in value]
        if isinstance(value, tuple):
            return tuple(self.value(v) for v in value)
        if isinstance(value, dict):
            return {k: "<elapsed>" if k in WALL_CLOCK_KEYS else self.value(v) for k, v in value.items()}
        return value


# Export keys the shared report design changes (docs/reference/report-design.md).
DESIGN_KEYS = ("bytes", "pages", "sheets", "rows", "sections", "files", "renderer")


def mask_design(payload) -> set[str]:
    """Mask an export envelope's design-dependent keys in place.

    Returns the artifact digests it found, which callers mask in books too.
    """
    data = payload.get("data") if isinstance(payload, dict) else None
    if not isinstance(data, dict) or "file" not in data:
        return set()
    digests = set()
    snapshot = data.get("report_snapshot")
    if isinstance(snapshot, dict) and isinstance(snapshot.get("content_sha256"), str):
        digests.add(snapshot["content_sha256"])
    # A row count is the design's only for sectioned or sheeted reports; a
    # single-table export's count must match (as `_csv_design` treats it).
    layout = "sections" in data or "sheets" in data
    for key in DESIGN_KEYS:
        if key in data and (key != "rows" or layout):
            data[key] = "<design>"
    if isinstance(data.get("file"), str):
        data["file"] = data["file"].removesuffix(".zip").removesuffix(".csv")
    return digests


def canonical(value):
    """``value`` with every list sorted and JSON text parsed, recursively.

    For runs that each create their own rows: random identities decide the
    order of id-keyed lists and the digests over them, so only the multiset
    of masked values can be compared.
    """
    if isinstance(value, str) and value[:1] in "[{":
        try:
            return ("<json>", canonical(json.loads(value)))
        except ValueError:
            return value
    if isinstance(value, dict):
        return {key: canonical(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return sorted((canonical(item) for item in value), key=lambda item: json.dumps(item, sort_keys=True, default=repr))
    return value


def dump(root: Path) -> dict:
    """Every manifest table with typed values (floats as hex), read-only."""
    from tests.rust_port.book_schema import TABLES
    from tests.test_rust_store_migrations import normalized_dump

    conn = sqlite3.connect((root / "kassiber.sqlite3").as_uri() + "?mode=ro", uri=True)
    try:
        _, tables = normalized_dump(conn)
    finally:
        conn.close()
    names = tuple(sorted(name for name in tables if not name.startswith("sqlite_")))
    assert names == TABLES, names
    return {name: tables[name] for name in names}


def book_identities(root: Path) -> set[str]:
    found: set[str] = set()
    for _, (_, rows) in dump(root).items():
        for row in rows:
            for _, value in row:
                if isinstance(value, str):
                    found.update(UUID.findall(value))
    return found


def masked_book(root: Path, masker: Masker) -> dict:
    """The dump with ``masker`` applied, each table's rows re-sorted."""
    masked = {}
    for table, (columns, rows) in dump(root).items():
        masked[table] = (columns, sorted(
            (tuple((kind, masker.value(value)) for kind, value in row) for row in rows), key=repr))
    return masked


def envelope(result) -> dict:
    stdout = result.stdout if isinstance(result.stdout, str) else result.stdout.decode()
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as exc:
        stderr = result.stderr if isinstance(result.stderr, str) else result.stderr.decode()
        raise AssertionError(
            f"not a JSON envelope (exit {result.returncode})\nstdout={stdout}\nstderr={stderr}") from exc
