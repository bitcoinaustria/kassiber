#!/usr/bin/env python3
"""Phase profile of Python's ``journals process`` on a disposable book copy.

Each run copies the book's data root, starts a fresh interpreter, and runs the
real CLI entry point (``kassiber --machine journals process``) with wall-clock
wrappers around the phases of the journal path: snapshot SQL, the custody
stages, the ledger request, the Rust ledger engine, response rendering, the
balance and ownership joins, and persistence. Wrappers only time calls; they
change no argument or result. After the timed command the child also times
the Rust custody binding on the same snapshot, so both engines see one input.

    .venv/bin/python scripts/benchmark-journals-phases.py \
        --book /path/to/book --runs 3 --scratch target/r4-bench

``--book`` is a directory holding ``data/kassiber.sqlite3`` (plaintext). It is
never modified. ``--snapshot-out`` also writes the raw custody request, for
``cargo run --example custody-bench``. Times are observations on the local
machine; compare engines back to back on a quiet machine.
"""

from __future__ import annotations

import time

_PROCESS_START = time.perf_counter()

import argparse
import contextlib
import functools
import importlib
import io
import json
import os
import shutil
import statistics
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SCOPE = ["--workspace", "Main", "--profile", "Default"]


class PhaseTimer:
    """Inclusive wall time per call path; recursion into a timed label is not re-counted."""

    def __init__(self) -> None:
        self.stack: list[str] = []
        self.records: dict[tuple[str, ...], list[float]] = {}

    def add(self, path: tuple[str, ...], elapsed: float, count: int = 1) -> None:
        record = self.records.setdefault(path, [0.0, 0])
        record[0] += elapsed
        record[1] += count

    def wrap(self, label: str, function):
        @functools.wraps(function)
        def timed(*args, **kwargs):
            if label in self.stack:
                return function(*args, **kwargs)
            path = (*self.stack, label)
            self.stack.append(label)
            start = time.perf_counter()
            try:
                return function(*args, **kwargs)
            finally:
                self.add(path, time.perf_counter() - start)
                self.stack.pop()

        return timed


# (module, attribute, label). Attributes are patched where they are looked up
# at call time, so module globals and class attributes are both covered.
WRAPS = [
    ("kassiber.cli.main", "build_parser", "build_parser"),
    ("kassiber.cli.main", "dispatch", "dispatch"),
    ("kassiber.cli.main", "process_journals", "process_journals"),
    ("kassiber.cli.main", "emit", "emit"),
    ("kassiber.core.custody_journal", "resolve_scope", "resolve_scope"),
    # --auto-pair: the desktop button and the freshness journal job.
    ("kassiber.daemon_freshness", "_journals_process_payload", "rebuild"),
    ("kassiber.core.custody_journal", "projection_freshness", "freshness"),
    ("kassiber.daemon_freshness", "suggest_transfer_candidates", "auto_pair_suggest"),
    ("kassiber.daemon_freshness", "projection_pairing_guards", "auto_pair_guards"),
    ("kassiber.daemon_freshness", "apply_transfer_rules", "auto_pair_rules"),
    ("kassiber.daemon_freshness", "bulk_pair_transfers", "auto_pair_bulk"),
    ("kassiber.daemon_freshness", "cache_swap_candidate_count", "auto_pair_cache"),
    ("kassiber.cli.handlers", "_repair_journal_source_overlaps", "repair_source_overlaps"),
    ("kassiber.cli.handlers", "_journal_source_overlap_warning", "source_overlap_warning"),
    ("kassiber.core.custody_authored_migration", "refresh_drifted_legacy_pair_components", "legacy_component_refresh"),
    ("kassiber.cli.handlers", "auto_price_transactions_from_rates_cache", "auto_price"),
    ("kassiber.core.custody_journal", "build_ledger_state", "build"),
    ("kassiber.core.custody_journal", "CustodyJournalBuilder.load_snapshot", "snapshot_load"),
    ("kassiber.core.book_network", "require_book_accounting", "require_book_accounting"),
    ("kassiber.core.custody_snapshot", "_network_facts", "network_facts"),
    ("kassiber.core.custody_journal", "CustodyJournalBuilder._transactions", "transactions_sql"),
    ("kassiber.core.custody_snapshot", "_safe_row", "safe_row"),
    ("kassiber.core.ownership", "build_owned_index", "owned_index"),
    ("kassiber.core.custody_gaps", "load_gap_search_worklist", "gap_worklist_sql"),
    ("kassiber.core.custody_journal", "transaction_rate_rows", "rate_rows_sql"),
    ("kassiber.core.custody_gap_reviews", "latest_review_rows", "gap_reviews_sql"),
    ("kassiber.core.source_overlap", "_receipt_overlap_candidate_groups", "receipt_groups"),
    ("kassiber.core.source_overlap", "build_profile_source_index", "receipt_source_index"),
    ("kassiber.core.custody_compute", "compute_custody_projection", "custody_compute"),
    ("kassiber.core.engines.ledger", "KassiberTaxEngine.build_ledger_state", "ledger"),
    ("kassiber.core.engines.ledger", "build_request", "request_build"),
    ("kassiber.core.engines.ledger", "dumps", "request_dumps"),
    ("kassiber_tax", "ledger", "engine_rust"),
    ("kassiber.core.engines.ledger", "loads", "response_loads"),
    ("kassiber.core.engines.ledger", "read_ledger", "response_render"),
    ("kassiber.core.engines.ledger_response", "rows_after_carry", "carry_rows"),
    ("kassiber.core.engines.ledger_response", "render_audit", "intra_audit"),
    ("kassiber.core.engines.ledger_response", "_EntryReader.entry", "entries"),
    ("kassiber.core.engines.ledger_response", "accumulate_holdings", "holdings"),
    ("kassiber.core.engines.ledger_response", "render_quarantine", "quarantines"),
    ("kassiber.core.engines.ledger_response", "direct_payout_compat_records", "direct_payouts"),
    ("kassiber.core.engines.ledger_response", "render_tax_summary", "tax_summary"),
    ("kassiber.core.custody_quantity_runtime", "compare_wallet_balances", "balance_join"),
    ("kassiber.core.custody_journal", "ownership_review_projection", "ownership_join"),
    ("kassiber.core.custody_journal", "store_ledger_state", "store"),
    ("kassiber.core.custody_reconciliation_store", "replace_reconciliations", "reconciliations"),
    ("kassiber.core.custody_quantity_store", "replace_canonical_quantity_state", "quantity_state"),
    ("kassiber.core.pricing", "journal_exact_payload", "entry_exact_payload"),
    ("kassiber.core.tax_events", "dedupe_quarantines", "quarantine_dedupe"),
    ("kassiber.core.custody_journal", "_quarantine_detail_with_ownership_candidates", "quarantine_detail"),
    ("kassiber.core.custody_filed_reports", "resolve_pending_custody_impacts", "filed_impacts"),
    ("kassiber.core.filed_report_chain", "resolve_pending_chain_impacts", "filed_chain_impacts"),
]


def install(timer: PhaseTimer) -> None:
    for module_name, attribute, label in WRAPS:
        module = importlib.import_module(module_name)
        owner, _, name = attribute.rpartition(".")
        target = getattr(module, owner) if owner else module
        setattr(target, name, timer.wrap(label, getattr(target, name)))


class StageObserver:
    """Turn the custody build observer's stage callbacks into stage spans."""

    def __init__(self, timer: PhaseTimer, sink: dict) -> None:
        self.timer = timer
        self.sink = sink
        self.path = tuple(timer.stack)
        self.last = time.perf_counter()

    def stage(self, name, value):
        now = time.perf_counter()
        # "snapshot" ends the SQL load; later names end each compute stage. The
        # first compute span also holds row capture, rates and channel context.
        self.timer.add((*self.path, "custody_stages", name), now - self.last)
        if name == "snapshot":
            self.sink["snapshot"] = value
        self.last = time.perf_counter()

    def owned_index(self, index):
        return index

    def fail(self, exc):
        pass

    def finish(self, result):
        self.timer.add((*self.path, "custody_stages", "decisions_return"), time.perf_counter() - self.last)


def child(args) -> None:
    timer = PhaseTimer()
    import kassiber.cli.main as cli_main
    import kassiber_tax  # noqa: F401
    from kassiber.core import custody_journal

    imported = time.perf_counter()
    install(timer)
    sink: dict = {}
    custody_journal.install_build_observer_factory(lambda builder: StageObserver(timer, sink))
    argv = ["--machine", "--data-root", str(args.data_root), "journals", "process", *SCOPE]
    if args.auto_pair:
        argv.append("--auto-pair")
    stdout = io.StringIO()
    start = time.perf_counter()
    with contextlib.redirect_stdout(stdout):
        code = cli_main.main(argv)
    command_end = time.perf_counter()
    command = command_end - start
    envelope = json.loads(stdout.getvalue())
    if code != 0:
        raise SystemExit(f"journals process failed: {stdout.getvalue()[:800]}")
    snapshot = sink["snapshot"]

    # The Rust custody binding on the very same raw request, outside the timed command.
    start = time.perf_counter()
    text = json.dumps(snapshot, ensure_ascii=True, allow_nan=True)
    dumps_time = time.perf_counter() - start
    rust_binding = None
    if args.rust_runs > 0:
        rust = []
        for _ in range(args.rust_runs):
            start = time.perf_counter()
            response = kassiber_tax.custody(text)
            rust.append(time.perf_counter() - start)
        start = time.perf_counter()
        decoded = json.loads(response)
        loads_time = time.perf_counter() - start
        if "error" in decoded:
            raise SystemExit(f"rust custody refused: {decoded['error']}")
        rust_binding = {
            "snapshot_dumps_seconds": dumps_time,
            "custody_seconds_best": min(rust),
            "response_bytes": len(response),
            "response_loads_seconds": loads_time,
        }
    if args.snapshot_out:
        Path(args.snapshot_out).write_text(text)
    report = {
        "import_seconds": imported - _PROCESS_START,
        "command_seconds": command,
        "to_command_end_seconds": command_end - _PROCESS_START,
        "phases": [[list(path), seconds, count] for path, (seconds, count) in timer.records.items()],
        "result": {key: envelope["data"].get(key) for key in ("entries_created", "quarantined", "processed_transactions", "cross_asset_pairs", "transfers_detected", "rebuilt")},
        "snapshot_bytes": len(text),
        "rust_binding": rust_binding,
    }
    Path(args.report).write_text(json.dumps(report))


def with_parents(records) -> dict[tuple[str, ...], tuple[float, int]]:
    """Phases keyed by path, adding untimed groups as the sum of their children."""

    phases = {tuple(path): (seconds, count) for path, seconds, count in records}
    for path in sorted(phases, key=len, reverse=True):
        parent = path[:-1]
        if parent and parent not in phases:
            phases[parent] = (sum(s for p, (s, _) in phases.items() if p[:-1] == parent), 1)
    return phases


def render(runs: list[dict], boot: float) -> str:
    def total(run):
        return boot + run["to_command_end_seconds"]

    best = min(runs, key=total)
    phases = with_parents(best["phases"])
    per_phase: dict[tuple[str, ...], list[float]] = {}
    for run in runs:
        for path, (seconds, _) in with_parents(run["phases"]).items():
            per_phase.setdefault(path, []).append(seconds)
    lines = [
        f"end to end (interpreter boot + imports + command), best of {len(runs)}: {total(best):.3f} s; "
        f"median {statistics.median(total(r) for r in runs):.3f} s",
        f"  bare interpreter boot (python -c pass): {boot:.3f} s",
        f"  imports after boot: {best['import_seconds']:.3f} s",
        f"  journals process command:    {best['command_seconds']:.3f} s",
        f"  result: {best['result']}",
        "phase (best run; median over runs in brackets)",
    ]

    def visit(prefix: tuple[str, ...], depth: int) -> None:
        children = [path for path in phases if len(path) == len(prefix) + 1 and path[: len(prefix)] == prefix]
        children.sort(key=lambda path: -phases[path][0])
        for path in children:
            seconds, count = phases[path]
            median = statistics.median(per_phase[path])
            calls = f" x{count}" if count > 1 else ""
            lines.append(f"{'  ' * depth}{path[-1]:<{44 - 2 * depth}} {seconds:8.3f} s  [{median:7.3f}]{calls}")
            visit(path, depth + 1)

    visit((), 1)
    rust = best["rust_binding"]
    if rust is None:
        lines.append(f"snapshot {best['snapshot_bytes'] / 1e6:.1f} MB")
        return "\n".join(lines)
    lines += [
        f"snapshot {best['snapshot_bytes'] / 1e6:.1f} MB; Rust custody response {rust['response_bytes'] / 1e6:.1f} MB",
        f"  Python json.dumps(snapshot): {rust['snapshot_dumps_seconds']:.3f} s",
        f"  kassiber_tax.custody (Rust, text in, text out), best: {rust['custody_seconds_best']:.3f} s",
        f"  Python json.loads(response): {rust['response_loads_seconds']:.3f} s",
    ]
    return "\n".join(lines)


def parent(args) -> None:
    book = Path(args.book).resolve()
    if not (book / "data" / "kassiber.sqlite3").is_file():
        raise SystemExit(f"{book} has no data/kassiber.sqlite3")
    scratch = Path(args.scratch).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "KASSIBER_DISABLE_UPDATE_CHECK": "1", "KASSIBER_NO_EGRESS": "1"}
    boots = []
    for _ in range(5):
        start = time.perf_counter()
        subprocess.run([sys.executable, "-c", "pass"], env=env, check=True)
        boots.append(time.perf_counter() - start)
    boot = min(boots)
    runs = []
    for index in range(args.runs):
        work = scratch / f"run-{index}"
        shutil.rmtree(work, ignore_errors=True)
        shutil.copytree(book, work, ignore=shutil.ignore_patterns("*.csv"))
        report = work / "report.json"
        command = [sys.executable, __file__, "--child", "--data-root", str(work / "data"),
                   "--report", str(report), "--rust-runs", str(args.rust_runs)]
        if args.auto_pair:
            command.append("--auto-pair")
        if args.snapshot_out and index == 0:
            command += ["--snapshot-out", args.snapshot_out]
        start = time.perf_counter()
        subprocess.run(command, cwd=REPO_ROOT, env=env, check=True)
        wall = time.perf_counter() - start
        run = json.loads(report.read_text())
        run["wall_seconds"] = wall
        runs.append(run)
        shutil.rmtree(work, ignore_errors=True)
    text = render(runs, boot)
    print(f"book: {book}\nload average: {os.getloadavg()}\n{text}", flush=True)
    if args.out:
        with open(args.out, "a") as handle:
            handle.write(json.dumps({"book": str(book), "load": os.getloadavg(), "boot_seconds": boot, "runs": runs}) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--book")
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--rust-runs", type=int, default=3, help="binding runs on the snapshot; 0 skips them")
    parser.add_argument("--scratch", default=str(REPO_ROOT / "target" / "r4-bench"))
    parser.add_argument("--out")
    parser.add_argument("--snapshot-out")
    parser.add_argument("--auto-pair", action="store_true", help="profile `journals process --auto-pair`")
    parser.add_argument("--child", action="store_true")
    parser.add_argument("--data-root")
    parser.add_argument("--report")
    args = parser.parse_args()
    if args.child:
        child(args)
    else:
        if not args.book:
            parser.error("--book is required")
        parent(args)


if __name__ == "__main__":
    main()
