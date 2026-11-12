"""Run every product CLI step on Python and Rust from the same state (test code only).

`integration-harness.sh` with `KASSIBER_RUNTIME=shadow` points
`KASSIBER_TEST_CLI` here. For each invocation that names a `--data-root`:

1. the book is copied to a mirror root, so both runtimes start from one state;
2. Python runs on the real book, then Rust (`KASSIBER_RUST_BIN`) on the
   mirror, against the same live services;
3. both envelopes and every table of the store's schema manifest are compared
   with fresh identities and clocks masked (`runtime_parity.py`), falling
   back to an identity-blind, order-blind comparison when random row ids
   decide an order; and
4. one JSON line per step goes to `KASSIBER_RUNTIME_SHADOW_LOG`, then
   Python's exit status and output are returned, so the lane keeps running on
   the Python reference.

Paths under the data root are rewritten to the mirror for Rust; an output
path that does not exist yet is redirected into the shadow directory so the
Rust artifact cannot replace Python's. Secret file descriptors are read once
and replayed to each runtime through fresh pipes. `daemon` sessions and calls
without a data root run on Python alone.

Live services see each step twice. Steps whose second run observes the
first's side effect (a Core wallet already holding the imported descriptors)
are expected to differ only in those counters; the log keeps both outcomes.
"""
from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

if __package__ in (None, ""):  # run as a script: `python tests/integration/runtime_shadow.py`
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from tests.integration.runtime_parity import (  # noqa: E402
    Masker, book_identities, canonical, dump, mask_design, rust_guard_capable,
)

ROOT = Path(__file__).resolve().parents[2]
# Flags whose value may name a file the command writes.
OUTPUT_FLAGS = frozenset({"--file", "--output", "--out", "--output-file", "--output-dir", "--directory",
                          "--dir", "--destination", "--bundle", "--json-output"})
SECRET_FLAG = re.compile(r"password|passphrase|token|secret|macaroon|rune|api-key|auth", re.I)


def _python() -> list[str]:
    from tests import cli_runner

    return list(cli_runner.prefix(reference=True))


def _data_root(argv: list[str]) -> Path | None:
    for index, arg in enumerate(argv):
        if arg == "--data-root" and index + 1 < len(argv):
            return Path(argv[index + 1])
        if arg.startswith("--data-root="):
            return Path(arg.split("=", 1)[1])
    return None


def _read_fd_args(argv: list[str]) -> dict[int, bytes]:
    """Content of every descriptor a `--*-fd N` flag names, read once."""
    contents: dict[int, bytes] = {}
    for index, arg in enumerate(argv[:-1]):
        if arg.startswith("--") and arg.endswith("-fd") and argv[index + 1].isdigit():
            fd = int(argv[index + 1])
            chunks = []
            while True:
                chunk = os.read(fd, 65536)
                if not chunk:
                    break
                chunks.append(chunk)
            os.close(fd)
            contents[fd] = b"".join(chunks)
    return contents


def _with_fds(argv: list[str], contents: dict[int, bytes]) -> tuple[list[str], list[int]]:
    mapped, read_ends = list(argv), []
    for index, arg in enumerate(argv[:-1]):
        if arg.startswith("--") and arg.endswith("-fd") and argv[index + 1].isdigit():
            data = contents.get(int(argv[index + 1]), b"")
            read_end, write_end = os.pipe()
            os.write(write_end, data)
            os.close(write_end)
            read_ends.append(read_end)
            mapped[index + 1] = str(read_end)
    return mapped, read_ends


def _redacted(argv: list[str]) -> list[str]:
    out, hide = [], False
    for arg in argv:
        out.append("<redacted>" if hide else arg)
        hide = arg.startswith("--") and bool(SECRET_FLAG.search(arg)) and not arg.endswith("-fd")
    return out


def _rust_argv(argv: list[str], root: Path, mirror: Path, outputs: Path) -> tuple[list[str], dict[str, str]]:
    """Rust's argv and the path renames to undo when comparing."""
    # The book's siblings (managed attachments) move with it.
    renames: dict[str, str] = {str(mirror): str(root), str(mirror.parent): str(root.parent)}
    mapped = []
    root_text = str(root)
    for index, arg in enumerate(argv):
        if root_text in arg:
            # Paths under the data root, also inside JSON values such as a
            # wallet's `source_file`.
            mapped.append(arg.replace(root_text, str(mirror)))
            continue
        previous = argv[index - 1] if index else ""
        if previous in OUTPUT_FLAGS and "://" not in arg and not Path(arg).exists():
            target = outputs / hashlib.sha256(arg.encode()).hexdigest()[:16] / Path(arg).name
            target.parent.mkdir(parents=True, exist_ok=True)
            renames[str(target)] = arg
            # An export may swap the suffix (the design's `.csv` → `.zip`).
            renames[str(target.with_suffix(""))] = str(Path(arg).with_suffix(""))
            mapped.append(str(target))
            continue
        mapped.append(arg)
    return mapped, renames


def _envelope(stdout: bytes):
    try:
        return json.loads(stdout)
    except ValueError:
        return {"<non-json>": stdout.decode(errors="replace")[-4000:]}


def _unrename(value, renames: dict[str, str]):
    if isinstance(value, str):
        for actual, original in sorted(renames.items(), key=lambda kv: -len(kv[0])):
            value = value.replace(actual, original)
        return value
    if isinstance(value, list):
        return [_unrename(v, renames) for v in value]
    if isinstance(value, dict):
        return {k: _unrename(v, renames) for k, v in value.items()}
    return value


def _first_difference(left, right, at="") -> str:
    if type(left) is not type(right):
        return f"{at or '<root>'}: {type(left).__name__} != {type(right).__name__}"
    if isinstance(left, dict):
        for key in sorted(set(left) | set(right), key=str):
            if key not in left or key not in right:
                return f"{at}/{key}: only in {'rust' if key not in left else 'python'}"
            if left[key] != right[key]:
                return _first_difference(left[key], right[key], f"{at}/{key}")
    if isinstance(left, (list, tuple)):
        if len(left) != len(right):
            return f"{at}: length {len(left)} != {len(right)}"
        for index, (a, b) in enumerate(zip(left, right)):
            if a != b:
                return _first_difference(a, b, f"{at}[{index}]")
    return f"{at or '<root>'}: {str(left)[:300]!r} != {str(right)[:300]!r}"


def _compare(python, rust, known: set[str], now: datetime, digests: set[str] = frozenset()) -> tuple[str, str | None]:
    numbered_py, numbered_rs = Masker(known, now), Masker(known, now)
    for masker in (numbered_py, numbered_rs):
        masker.digests |= set(digests)
    left, right = numbered_py.value(python), numbered_rs.value(rust)
    if left == right:
        return "same", None
    def blind(value):
        masker = Masker(known, now, anonymous=True)
        masker.digests |= set(digests)
        return canonical(masker.value(value))
    if blind(python) == blind(rust):
        return "identity-ordered", None
    return "diff", _first_difference(blind(python), blind(rust))


def _book(root: Path, renames: dict[str, str]):
    """Every manifest table, unmasked; `_compare` masks both sides."""
    try:
        tables = dump(root)
    except Exception as exc:  # unreadable (encrypted) or absent books are reported, not compared
        return {"<unreadable>": type(exc).__name__}
    return _unrename({name: [columns, [[value for _, value in row] for row in rows]]
                      for name, (columns, rows) in tables.items()}, renames)


def main(argv: list[str]) -> int:
    root = _data_root(argv)
    rust_bin = os.environ.get("KASSIBER_RUST_BIN")
    if root is None or not rust_bin or "daemon" in argv:
        os.execv(sys.executable, [*_python(), *argv])
    shadow = Path(os.environ.get("KASSIBER_RUNTIME_SHADOW_DIR") or (ROOT / "target" / "runtime-shadow"))
    key = hashlib.sha256(str(root).encode()).hexdigest()[:16]
    mirror = shadow / "books" / key / root.name
    outputs = shadow / "outputs" / key
    # Read stdin only for a command that consumes it; an inherited stdin may
    # be a pipe or socket that never closes. Otherwise both runs inherit it.
    wants_stdin = any(arg == "-" or (arg.startswith("--") and arg.endswith("-stdin")) for arg in argv)
    stdin = sys.stdin.buffer.read() if wants_stdin else None
    fds = _read_fd_args(argv)
    now = datetime.now(timezone.utc)

    if mirror.exists():
        shutil.rmtree(mirror)
    existed = (root / "kassiber.sqlite3").exists()
    if root.exists():
        shutil.copytree(root, mirror, symlinks=True)
    else:
        mirror.parent.mkdir(parents=True, exist_ok=True)
    known = book_identities(root) if existed else set()

    def run(command: list[str], env: dict[str, str]):
        mapped, read_ends = _with_fds(command, fds)
        started = time.perf_counter()
        try:
            result = subprocess.run(mapped, input=stdin, capture_output=True, env=env, cwd=os.getcwd(),
                                    pass_fds=read_ends, timeout=3600)
        finally:
            for fd in read_ends:
                os.close(fd)
        return result, round((time.perf_counter() - started) * 1000)

    # Decide Rust's paths before Python writes anything: an output Python is
    # about to create must not look like an existing input afterwards.
    rust_args, renames = _rust_argv(argv, root, mirror, outputs)
    python, python_ms = run([*_python(), *argv], dict(os.environ))
    rust_env = {k: v for k, v in os.environ.items() if k != "KASSIBER_TEST_CLI"}
    if not rust_guard_capable(rust_bin):
        rust_env.pop("KASSIBER_TEST_NO_EGRESS", None)
    rust, rust_ms = run([rust_bin, *rust_args], rust_env)

    python_payload, rust_payload = _envelope(python.stdout), _unrename(_envelope(rust.stdout), renames)
    digests: set[str] = set()
    for payload in (python_payload, rust_payload):
        digests |= mask_design(payload)
    envelope, envelope_diff = _compare(python_payload, rust_payload, known, now, digests)
    book, book_diff = "absent", None
    if (root / "kassiber.sqlite3").exists() or (mirror / "kassiber.sqlite3").exists():
        book, book_diff = _compare(_book(root, {}), _book(mirror, renames), known, now, digests)
    log = os.environ.get("KASSIBER_RUNTIME_SHADOW_LOG")
    if log:
        record = {
            "argv": _redacted(argv), "python_rc": python.returncode, "rust_rc": rust.returncode,
            "python_ms": python_ms, "rust_ms": rust_ms, "envelope": envelope, "book": book,
        }
        if envelope_diff:
            record["envelope_diff"] = envelope_diff
        for name, result in (("python", python), ("rust", rust)):
            if result.returncode:
                payload = _envelope(result.stdout)
                error = payload.get("error") if isinstance(payload, dict) else None
                record[f"{name}_error"] = (
                    {key: error.get(key) for key in ("code", "message", "details")}
                    if isinstance(error, dict) else result.stderr.decode(errors="replace")[-2000:]
                )
        if book_diff:
            record["book_diff"] = book_diff
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")
    sys.stdout.buffer.write(python.stdout)
    sys.stderr.buffer.write(python.stderr)
    return python.returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
