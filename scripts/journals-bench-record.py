#!/usr/bin/env python3
"""Time Python and Rust ``journals process`` back to back and append a record.

    uv run --locked python scripts/journals-bench-record.py \\
        --book target/r4-bench/books/generic-20000 --step R4-1 --runs 2 \\
        --rust target/release/examples/journals-bench \\
        --record target/r4-bench/records.jsonl

``--book`` is a book from ``scripts/journals-bench-books.py``. Each run pair
times Python first (``scripts/benchmark-journals-phases.py``: a fresh
interpreter on a fresh copy of the book) and then the Rust pipeline
(``journals-bench``, built with ``--features timing``) on the snapshot that
Python's first run loaded. Python and Rust therefore always see one input and
the same machine state.

Before each pair the 1-minute load average must be at most ``--max-load``.
While it is higher the recorder waits; after ``--wait-minutes`` it measures
anyway, takes the best of at least five alternating pairs, and marks the
record ``"policy": "loaded"``. Every pair records its load.

The record is one JSON line (schema ``kassiber.journals-bench.v1``) holding
the book manifest, the commit, both engines' best runs with every phase and
span, the ledger hashes, and comparable lines derived from both
(docs/reference/r4-journal-pipeline.md#bench-records). ``--rust-before BIN``
times the previous step's bench in the same pairs, so a step's before and after
are back to back; both must report the same ledger hashes. ``--baseline FILE``
prints each line against the latest record of the same book in that file.
Records hold synthetic books only; owner-book timings stay local.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
PROFILER = REPO_ROOT / "scripts" / "benchmark-journals-phases.py"
SCHEMA = "kassiber.journals-bench.v1"
LOADED_RUNS = 5

# Comparable lines: (name, Python phase paths, Rust span names). Python paths
# are relative to `dispatch/process_journals`; a missing phase counts as zero.
LINES = (
    ("snapshot in (Python SQL load; Rust JSON parse until R4-9)", ("build/snapshot_load",), ("parse",)),
    ("custody (admission through finalized projection)", ("build/custody_compute",), (
        "admission", "boundary", "components+migration", "evidence", "interpretation", "gaps", "quantity", "finalized_projection",
    )),
    ("fusion request", ("build/ledger/request_build", "build/ledger/request_dumps"), ("fusion.input", "fusion.request", "fusion.text")),
    ("tax engine", ("build/ledger/engine_rust",), ("engine",)),
    ("render and joins", ("build/ledger/response_loads", "build/ledger/response_render", "build/balance_join", "build/ownership_join"), ()),
    ("persistence", ("store",), ()),
    ("Rust JSON parse and admission (R4-2 target)", (), ("parse", "admission")),
)


def load_average() -> list[float]:
    return [round(value, 2) for value in os.getloadavg()]


def wait_for_quiet(max_load: float, wait_minutes: float) -> bool:
    """Whether the machine became quiet within the wait."""
    deadline = time.monotonic() + wait_minutes * 60
    while os.getloadavg()[0] > max_load:
        if time.monotonic() >= deadline:
            return False
        print(f"load {load_average()} above {max_load}; waiting", file=sys.stderr, flush=True)
        time.sleep(30)
    return True


def python_run(book: Path, scratch: Path, snapshot_out: Path | None) -> dict:
    out = scratch / "python-run.jsonl"
    out.unlink(missing_ok=True)
    command = [sys.executable, str(PROFILER), "--book", str(book), "--runs", "1", "--rust-runs", "0",
               "--scratch", str(scratch), "--out", str(out)]
    if snapshot_out is not None:
        command += ["--snapshot-out", str(snapshot_out)]
    subprocess.run(command, cwd=REPO_ROOT, check=True, stdout=subprocess.DEVNULL)
    record = json.loads(out.read_text().splitlines()[-1])
    run = record["runs"][0]
    # The profiler's end-to-end time: a bare interpreter boot plus everything
    # up to the command's end, as its own report renders it.
    run["end_to_end_seconds"] = record["boot_seconds"] + run["to_command_end_seconds"]
    return run


def rust_run(binary: Path, snapshot: Path) -> dict:
    result = subprocess.run([str(binary), str(snapshot), "--runs", "1", "--json"], cwd=REPO_ROOT,
                            check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def phases(run: dict) -> dict[str, float]:
    """Python phases keyed by their path under `dispatch/process_journals`, plus top-level ones."""
    found: dict[str, float] = {}
    for path, seconds, _count in run["phases"]:
        if path[:2] == ["dispatch", "process_journals"] and len(path) > 2:
            key = "/".join(path[2:])
        else:
            key = "/".join(path)
        found[key] = found.get(key, 0.0) + seconds
    return found


def spans(run: dict) -> dict[str, float]:
    return {name: seconds for name, seconds, _count in run["spans"]}


def lines(python: dict, rust: dict, before: dict | None) -> list[dict]:
    py = phases(python)

    def total(spans_: dict, names) -> float | None:
        return round(sum(spans_.get(name, 0.0) for name in names), 4) if names else None

    result = []
    for name, py_paths, rs_names in LINES:
        line = {
            "line": name,
            "python": round(sum(py.get(path, 0.0) for path in py_paths), 4) if py_paths else None,
            "rust": total(spans(rust), rs_names),
        }
        if before is not None:
            line["rust_before"] = total(spans(before), rs_names)
        result.append(line)
    return result


def rust_summary(runs: list[dict]) -> dict:
    best = min(runs, key=lambda run: run["compute_seconds"])
    return {
        "runs_compute": [round(run["compute_seconds"], 4) for run in runs],
        "compute_seconds": best["compute_seconds"],
        "spans": spans(best),
        "fusion": best["fusion"],
        "ledger_request_sha256": best["ledger_request_sha256"],
        "ledger_response_sha256": best["ledger_response_sha256"],
    }


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=REPO_ROOT, capture_output=True, text=True).stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--book", required=True)
    parser.add_argument("--step", required=True, help="the R4 step this record measures, such as R4-2")
    parser.add_argument("--rust", required=True, help="the journals-bench example binary")
    parser.add_argument("--rust-before", help="the previous step's journals-bench, timed in the same pairs")
    parser.add_argument("--record", required=True, help="JSON lines file to append to")
    parser.add_argument("--runs", type=int, default=3, help="run pairs on a quiet machine")
    parser.add_argument("--max-load", type=float, default=1.5)
    parser.add_argument("--wait-minutes", type=float, default=30)
    parser.add_argument("--scratch", default=str(REPO_ROOT / "target" / "r4-bench" / "scratch"))
    parser.add_argument("--baseline", help="records file to compare with")
    parser.add_argument("--note", default="", help="free text stored with the record")
    args = parser.parse_args()
    book = Path(args.book).resolve()
    manifest = json.loads((book / "bench-book.json").read_text())
    scratch = Path(args.scratch).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    snapshot = scratch / f"snapshot-{book.name}.json"

    # The checkout under test, before any waiting.
    commit = git("rev-parse", "HEAD")
    dirty = bool(git("status", "--porcelain", "--untracked-files=no"))
    quiet = wait_for_quiet(args.max_load, args.wait_minutes)
    pairs = []
    while len(pairs) < (args.runs if quiet else max(args.runs, LOADED_RUNS)):
        if quiet and pairs and not wait_for_quiet(args.max_load, args.wait_minutes):
            quiet = False
        load = load_average()
        python = python_run(book, scratch, snapshot if not pairs else None)
        before = rust_run(Path(args.rust_before).resolve(), snapshot) if args.rust_before else None
        rust = rust_run(Path(args.rust).resolve(), snapshot)
        pairs.append({"load": load, "python": python, "rust": rust, "rust_before": before})
        print(f"pair {len(pairs)}: load {load} python {python['end_to_end_seconds']:.3f} s "
              f"rust compute {rust['compute_seconds']:.3f} s"
              + (f", before {before['compute_seconds']:.3f} s" if before else ""), file=sys.stderr, flush=True)
    python = min((pair["python"] for pair in pairs), key=lambda run: run["end_to_end_seconds"])
    rust = min((pair["rust"] for pair in pairs), key=lambda run: run["compute_seconds"])
    before = min((pair["rust_before"] for pair in pairs), key=lambda run: run["compute_seconds"]) if args.rust_before else None
    ledgers = {
        (run["fusion"], run["ledger_request_sha256"], run["ledger_response_sha256"])
        for pair in pairs for run in (pair["rust"], pair["rust_before"]) if run is not None
    }
    if len(ledgers) != 1:
        raise SystemExit(f"Rust runs disagree on the ledger: {ledgers}")
    baseline = None
    if args.baseline and Path(args.baseline).is_file():
        for line in Path(args.baseline).read_text().splitlines():
            candidate = json.loads(line)
            if candidate.get("schema") == SCHEMA and candidate["book"]["name"] == book.name:
                baseline = candidate
    record = {
        "schema": SCHEMA,
        "step": args.step,
        "note": args.note,
        "commit": commit,
        "dirty": dirty,
        "recorded_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "machine": {"cpus": os.cpu_count(), "platform": platform.platform(), "python": platform.python_version()},
        "book": {"name": book.name, **{key: manifest[key] for key in ("kind", "transactions", "seed", "csv_sha256", "pegs_paired")}},
        "policy": "quiet" if quiet else "loaded",
        "max_load": args.max_load,
        "loads": [pair["load"] for pair in pairs],
        "python": {
            "runs_end_to_end": [round(pair["python"]["end_to_end_seconds"], 4) for pair in pairs],
            "end_to_end_seconds": python["end_to_end_seconds"],
            "command_seconds": python["command_seconds"],
            "import_seconds": python["import_seconds"],
            "result": python["result"],
            "snapshot_bytes": python["snapshot_bytes"],
            "phases": phases(python),
        },
        "rust": rust_summary([pair["rust"] for pair in pairs]),
        "rust_before": rust_summary([pair["rust_before"] for pair in pairs]) if before else None,
        "lines": lines(python, rust, before),
    }
    with open(args.record, "a") as handle:
        handle.write(json.dumps(record) + "\n")

    def cell(value) -> str:
        return f"{value:10.3f}" if value is not None else f"{'-':>10}"

    print(f"{book.name} ({args.step}, {record['policy']}, loads {record['loads']})")
    header = f"  {'line':58} {'python':>10} {'rust':>10}"
    if before:
        header += f" {'before':>10}"
    if baseline:
        header += f" {'baseline':>10}"
    print(header)
    base_lines = {line["line"]: line for line in baseline["lines"]} if baseline else {}
    for line in record["lines"]:
        text = f"  {line['line']:58} {cell(line['python'])} {cell(line['rust'])}"
        if before:
            text += f" {cell(line['rust_before'])}"
        if baseline:
            text += f" {cell(base_lines.get(line['line'], {}).get('rust'))}"
        print(text)
    print(f"  python end to end {python['end_to_end_seconds']:.3f} s; rust compute {rust['compute_seconds']:.3f} s"
          + (f"; before {before['compute_seconds']:.3f} s" if before else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
