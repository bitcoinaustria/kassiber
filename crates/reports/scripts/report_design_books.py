#!/usr/bin/env python3
"""Build the report-design sample books and capture Python's report data.

    uv run --locked python crates/reports/scripts/report_design_books.py --out target/report-design

Two books, each built with this checkout's CLI from seeded CSVs and processed
with ``journals process`` like a user's book:

- ``austria``: an Austrian EUR book (moving average) whose history starts
  before the 2021-03-01 Neuvermögen cutoff, so Alt and Neu both appear;
- ``generic``: a generic EUR book (FIFO).

Both hold a hot and a cold Bitcoin wallet and a Liquid wallet with monthly
buys, sells, interest income, fee-paying transfers between owned wallets and
a few spends. For each book ``python-model.json`` records, per report, exactly
what Python's reports produce: ``summary`` (the summary model for the sample
period, the profile's tax settings and, for Austria, the E 1kv Kennzahl totals
of each tax year) and ``full`` (the full report's model). The Rust renderers
must reproduce these values. ``--capture-only`` re-records them for books
built earlier.

Runs offline on the disposable ``--out`` only. Every CLI child gets its own
XDG directories so nothing resolves the account's real app-data root.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import random
import shutil
import sys
from dataclasses import replace
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT))
from tests import cli_runner  # noqa: E402

STAMP = "2026-01-02T03:04:05Z"
PERIOD = {"start": "2024-01-01T00:00:00Z", "end": "2025-12-31T23:59:59Z"}
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
SCOPE = ["--workspace", "Main", "--profile", "Default"]
BOOKS = {
    "austria": {"seed": 21, "country": "at", "algorithm": "MOVING_AVERAGE_AT", "start": (2020, 6)},
    "generic": {"seed": 7, "country": "generic", "algorithm": "FIFO", "start": (2022, 1)},
}
WALLETS = {
    "Hot": ["--chain", "bitcoin"],
    "Cold": ["--chain", "bitcoin"],
    "Liquid": ["--source-file", "/dev/null", "--chain", "liquid", "--network", "liquidv1"],
}


def price(t: datetime.datetime) -> int:
    """A seed-free EUR price path from about 8k (2020) to about 90k (2025)."""
    months = (t.year - 2020) * 12 + t.month - 1
    base = 8000 * (1.035 ** months)
    wobble = 1 + 0.08 * ((months * 37) % 11 - 5) / 5
    return int(base * wobble)


def rows(seed: int, start: tuple[int, int]) -> dict[str, list[str]]:
    rng = random.Random(seed)
    out: dict[str, list[str]] = {wallet: [] for wallet in WALLETS}
    asset = {"Hot": "BTC", "Cold": "BTC", "Liquid": "LBTC"}
    held = {wallet: 0 for wallet in WALLETS}
    n = 0

    def add(wallet, t, direction, amount, fee, kind, description, txid=None):
        nonlocal n
        n += 1
        out[wallet].append(
            f"{t:%Y-%m-%dT%H:%M:%SZ},{txid or f'{kind}-{n}'},{direction},{asset[wallet]},"
            f"{amount / 1e8:.8f},{fee / 1e8:.8f},{price(t)},{description},{kind}"
        )

    year, month = start
    while (year, month) <= (2025, 12):
        day = datetime.datetime(year, month, 3, 9, 0)
        buy = rng.randint(2_000_000, 9_000_000)
        held["Hot"] += buy
        add("Hot", day, "inbound", buy, 0, "buy", "Monthly purchase")
        if month % 3 == 0:
            interest = rng.randint(20_000, 90_000)
            held["Liquid"] += interest
            add("Liquid", day + datetime.timedelta(days=9), "inbound", interest, 0, "interest", "Lending interest")
        if month % 4 == 1 and held["Hot"] > 10_000_000:
            amount, fee = held["Hot"] // 2, rng.randint(800, 4000)
            held["Hot"] -= amount + fee
            held["Cold"] += amount
            txid = f"{n + 1:064x}"
            when = day + datetime.timedelta(days=5)
            add("Hot", when, "outbound", amount + fee, fee, "transfer", "To cold storage", txid)
            add("Cold", when, "inbound", amount, 0, "transfer", "From hot wallet", txid)
        if month in (5, 11) and held["Cold"] > 20_000_000:
            amount, fee = held["Cold"] // 5, rng.randint(1500, 6000)
            held["Cold"] -= amount + fee
            add("Cold", day + datetime.timedelta(days=17), "outbound", amount + fee, fee, "sell", "Exchange sale")
        if month == 8 and held["Hot"] > 3_000_000:
            amount = rng.randint(500_000, 2_000_000)
            held["Hot"] -= amount
            add("Hot", day + datetime.timedelta(days=12), "outbound", amount, 0, "expense", "Hardware purchase")
        month += 1
        if month == 13:
            year, month = year + 1, 1
    return out


def build(name: str, spec: dict, out: Path) -> Path:
    book = out / "books" / name
    if book.exists():
        shutil.rmtree(book)
    book.mkdir(parents=True)
    root = book / "data"
    isolated = {key: str(book / "xdg" / key.lower()) for key in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME")}
    env = {**os.environ, **isolated, "KASSIBER_DISABLE_UPDATE_CHECK": "1", "KASSIBER_NO_EGRESS": "1"}

    def run(*args: str) -> dict:
        result = cli_runner.run(["--machine", "--data-root", str(root), *args], reference=True,
                                cwd=REPO_ROOT, env=env, capture_output=True, text=True)
        if result.returncode != 0:
            raise SystemExit(f"FAILED {' '.join(args)}: {result.stdout[:800]} {result.stderr[-800:]}")
        return json.loads(result.stdout)

    run("init")
    run("workspaces", "create", "Main")
    run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", spec["country"],
        "--gains-algorithm", spec["algorithm"], "Default")
    for wallet, extra in WALLETS.items():
        run("wallets", "create", *SCOPE, "--label", wallet, "--kind", "custom", *extra)
    for wallet, lines in rows(spec["seed"], spec["start"]).items():
        path = book / f"{wallet}.csv"
        path.write_text(HEADER + "\n" + "\n".join(lines) + "\n")
        run("wallets", "import-csv", *SCOPE, "--wallet", wallet, "--file", str(path))
    run("journals", "process", *SCOPE)
    return root


def capture(root: Path, country: str) -> dict:
    from kassiber.cli.handlers import _report_hooks
    from kassiber.core import reports
    from kassiber.db import open_db

    from unittest.mock import patch
    from gen_full_pdf_vectors import FixedDateTime, full_model

    conn = open_db(str(root))
    hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
    context = reports._require_report_context(conn, None, None, hooks)
    profile = dict(context.profile)
    # The full report's history ends at the report clock, as in its oracle.
    with patch.object(reports, "datetime", FixedDateTime):
        full = full_model(reports._build_full_report_context(conn, None, None, hooks, report_context=context))
    summary = reports.build_summary_pdf_report_data(
        conn, None, None, hooks, start=PERIOD["start"], end=PERIOD["end"],
        include_snapshot=True, report_context=context)
    model = {
        "summary": summary,
        "profile": {key: profile[key] for key in ("tax_country", "gains_algorithm", "tax_long_term_days",
                                                   "fiat_currency", "journal_input_version")},
        "kennzahlen": {},
    }
    if country == "at":
        for year in (2024, 2025):
            report = reports.report_austrian_e1kv(conn, None, None, hooks, tax_year=year, report_context=context)
            model["kennzahlen"][str(year)] = report["kennzahl_totals"]
    conn.close()
    return json.loads(json.dumps({"summary": model, "full": full}))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--capture-only", action="store_true",
                        help="record Python's models for books built earlier")
    args = parser.parse_args()
    out = Path(args.out).resolve()
    for key in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        os.environ[key] = str(out / "xdg" / key.lower())
    for name, spec in BOOKS.items():
        root = out / "books" / name / "data" if args.capture_only else build(name, spec, out)
        model = capture(root, spec["country"])
        (out / "books" / name / "python-model.json").write_text(
            json.dumps(model, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
        print(name, root, flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
