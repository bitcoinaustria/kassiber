"""Compare what the Rust binary leaves in a book with a frozen record (test code only).

The parity tests once ran the same steps on the Python and the Rust runtime
and compared the two. The Python side is gone (docs/plan/22-python-removal.md,
D5): its outputs were frozen into goldens under `tests/integration/goldens/`
while it existed, and the tests now compare the Rust binary with them.
Envelopes and every table of the store's schema manifest must match once
fresh identities and clocks are masked. Standard library only.
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
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN") or None
GOLDENS = ROOT / "tests" / "integration" / "goldens"
# The current book schema (crates/store; fetched with scripts/fetch-test-data.sh).
SCHEMA_MANIFEST = ROOT / "crates" / "store" / "fixtures" / "current-schema.json"
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


def child_env(home: Path, *, rust_bin: str | None = None) -> dict[str, str]:
    """A disposable environment for the Rust binary's children.

    Preferences, XDG roots and HOME point under ``home``, so the child reads
    neither the account's offline switch nor its app-data root. The product
    kill switch is off because it refuses loopback for sync; the test socket
    guard blocks every non-loopback destination when ``rust_bin`` is a
    test-harness build. `KASSIBER_TEST_CLI` and `KASSIBER_RUST_BIN` pass
    through, so harness code run under this environment starts the same
    binary (`tests/integration/cli.py`).
    """
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("KASSIBER_OPERATOR_", "SATBOOKS_"))
        and key not in {"COLUMNS", "LINES", "KASSIBER_DATA_ROOT", "KASSIBER_STATE_ROOT", "KASSIBER_NO_EGRESS"}
    }
    env.update(
        {
            "HOME": str(home),
            "USERPROFILE": str(home),
            "LOCALAPPDATA": str(home),
            "XDG_DATA_HOME": str(home / "xdg-data"),
            "XDG_CONFIG_HOME": str(home / "xdg-config"),
            "XDG_STATE_HOME": str(home / "xdg-state"),
            "COLUMNS": "80",
            "LINES": "24",
            "KASSIBER_DISABLE_UPDATE_CHECK": "1",
            "KASSIBER_OFFLINE_PREFERENCE_FILE": str(home / "offline.json"),
            "KASSIBER_AGENT_ACCESS_FILE": str(home / "agents.json"),
            "KASSIBER_UPDATE_PREFERENCE_FILE": str(home / "updates.json"),
            "KASSIBER_UPDATE_CACHE_FILE": str(home / "update-cache.json"),
            "KASSIBER_STATE_ROOT": str(home / "state"),
            "TZ": "UTC",
        }
    )
    if rust_bin is not None and rust_guard_capable(rust_bin):
        env["KASSIBER_TEST_NO_EGRESS"] = "1"
    else:
        env.pop("KASSIBER_TEST_NO_EGRESS", None)
    return env


class Masker:
    """Fresh identities and clocks become stable placeholders.

    Identities in ``known`` (the shared starting book) stay literal. New ones
    are numbered in order of first appearance while reading envelopes; after
    ``freeze()`` any further new identity, such as a row key only the book
    holds, becomes ``<id>``. Clocks within two days of ``today`` and recorded
    artifact digests are masked too.
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


def schema_tables() -> tuple[str, ...]:
    """The current book schema's tables, from the store manifest."""
    manifest = json.loads(SCHEMA_MANIFEST.read_text(encoding="utf-8"))
    return tuple(sorted(obj[1] for obj in manifest["objects"] if obj[0] == "table"))


def _quote(text: str) -> str:
    return '"' + text.replace('"', '""') + '"'


def normalized_tables(conn: sqlite3.Connection) -> dict:
    """Every table's columns and rows in primary-key order, then all columns.

    Each value carries its SQLite storage class; floats are hex so the exact
    binary value is compared rather than Python's numeric equality.
    """
    tables = {}
    for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
        columns = list(conn.execute(f"PRAGMA table_info({_quote(table)})"))
        primary = [row[1] for row in sorted(columns, key=lambda row: row[5]) if row[5]]
        order = list(dict.fromkeys(primary + [row[1] for row in columns]))
        storage_classes = ",".join(f"typeof({_quote(row[1])})" for row in columns)
        rows = conn.execute(
            f"SELECT *,{storage_classes} FROM {_quote(table)} ORDER BY {','.join(map(_quote, order))}"
        ).fetchall()
        typed_rows = [
            tuple((kind, value.hex() if kind == "real" else value)
                  for kind, value in zip(row[len(columns):], row[:len(columns)]))
            for row in rows
        ]
        tables[table] = ([row[1] for row in columns], typed_rows)
    return tables


def dump(root: Path) -> dict:
    """Every manifest table with typed values (floats as hex), read-only."""
    conn = sqlite3.connect((root / "kassiber.sqlite3").as_uri() + "?mode=ro", uri=True)
    try:
        tables = normalized_tables(conn)
    finally:
        conn.close()
    names = tuple(sorted(name for name in tables if not name.startswith("sqlite_")))
    assert names == schema_tables(), names
    return {name: tables[name] for name in names}


def book_identities(root: Path) -> set[str]:
    found: set[str] = set()
    for _, (_, rows) in dump(root).items():
        for row in rows:
            for _, value in row:
                if isinstance(value, str):
                    found.update(UUID.findall(value))
    return found


def scrub_paths(value, base: Path):
    """``value`` with the run's scratch directory written as ``<base>``.

    A golden is recorded under another temporary directory than the run that
    compares with it; paths below the scratch root must match all the same.
    """
    prefix = str(base)
    if isinstance(value, str):
        return value.replace(prefix, "<base>")
    if isinstance(value, list):
        return [scrub_paths(item, base) for item in value]
    if isinstance(value, tuple):
        return tuple(scrub_paths(item, base) for item in value)
    if isinstance(value, dict):
        return {key: scrub_paths(item, base) for key, item in value.items()}
    return value


def masked_book(root: Path, masker: Masker, *, scrub: Path | None = None) -> dict:
    """The dump with ``masker`` applied, each table's rows re-sorted.

    ``scrub`` names a scratch directory whose paths become ``<base>`` first.
    """
    masked = {}
    for table, (columns, rows) in dump(root).items():
        if scrub is not None:
            rows = scrub_paths(rows, scrub)
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


def jsonable(value):
    """``value`` as JSON can hold it: tuples become lists, recursively.

    Goldens are JSON, so a live value is compared after the same round trip.
    """
    return json.loads(json.dumps(value, sort_keys=True, default=repr))


def load_golden(name: str):
    return json.loads((GOLDENS / name).read_text(encoding="utf-8"))


def write_golden(name: str, value) -> Path:
    GOLDENS.mkdir(parents=True, exist_ok=True)
    path = GOLDENS / name
    path.write_text(json.dumps(jsonable(value), indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return path
