#!/usr/bin/env python3
"""Build a synthetic bench book for ``journals process`` from a committed seed.

    uv run --locked python scripts/journals-bench-books.py \\
        --kind generic --transactions 20000 --out target/r4-bench/books/generic-20000

Two shapes, each drawn from Python's ``random`` with a fixed seed so the CSVs
are byte-identical across runs and machines:

- ``generic`` (seed 1): one custom BTC wallet with buys and sells.
- ``multi-wallet`` (seed 7): four custom wallets on Bitcoin and Liquid,
  same-txid transfers between the two Bitcoin wallets, Bitcoin-to-Liquid pegs,
  and spends. Up to 300 pegs (half of them on small books) are paired with
  ``transfers pair --kind peg-in``, which authors custody components.

The book is built with this checkout's CLI in ``--out``, with the data root
``<out>/data`` (the database is ``<out>/data/kassiber.sqlite3``), which is the
layout ``scripts/benchmark-journals-phases.py --book <out>`` profiles. It ends
with one ``journals process``, so the book holds a processed journal like a
user's. ``<out>/bench-book.json`` records the shape, seed, CSV digests and the
commit that built it. Runs offline (``KASSIBER_NO_EGRESS=1``) on the
disposable ``--out`` only; ``--force`` replaces an existing directory.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))
from tests import cli_runner

SCOPE = ["--workspace", "Main", "--profile", "Default"]
HEADER = "date,txid,direction,asset,amount,fee,fiat_rate,description,kind"
SEEDS = {"generic": 1, "multi-wallet": 7}
# Wallet label -> extra `wallets create` arguments.
MULTI_WALLETS = {
    "Hot": ["--chain", "bitcoin"],
    "Cold": ["--chain", "bitcoin"],
    "LHot": ["--source-file", "/dev/null", "--chain", "liquid", "--network", "liquidv1"],
    "LCold": ["--source-file", "/dev/null", "--chain", "liquid", "--network", "liquidv1"],
}
MAX_PAIRED_PEGS = 300


def generic_rows(count: int, seed: int) -> dict[str, list[str]]:
    rng = random.Random(seed)
    t = datetime.datetime(2020, 1, 1)
    held = 0
    lines = []
    for i in range(count):
        t += datetime.timedelta(minutes=rng.randint(30, 300))
        rate = 30000 + rng.randint(0, 40000)
        if held > 2_000_000 and rng.random() < 0.4:
            amount = rng.randint(10_000, held // 4)
            held -= amount
            lines.append(f"{t:%Y-%m-%dT%H:%M:%SZ},tx-{i},outbound,BTC,{amount / 1e8:.8f},0,{rate},Sell,sell")
        else:
            amount = rng.randint(100_000, 5_000_000)
            held += amount
            lines.append(f"{t:%Y-%m-%dT%H:%M:%SZ},tx-{i},inbound,BTC,{amount / 1e8:.8f},0,{rate},Buy,buy")
    return {"W1": lines}


def multi_wallet_rows(count: int, seed: int) -> tuple[dict[str, list[str]], list[tuple[str, str]]]:
    rng = random.Random(seed)
    rows: dict[str, list[str]] = {wallet: [] for wallet in MULTI_WALLETS}
    held = {wallet: 0 for wallet in MULTI_WALLETS}
    asset = {"Hot": "BTC", "Cold": "BTC", "LHot": "LBTC", "LCold": "LBTC"}
    pegs: list[tuple[str, str]] = []
    t = datetime.datetime(2020, 1, 1)
    written = 0

    def add(wallet, txid, direction, amount, fee, rate, kind, description):
        nonlocal written
        rows[wallet].append(
            f"{t:%Y-%m-%dT%H:%M:%SZ},{txid},{direction},{asset[wallet]},"
            f"{amount / 1e8:.8f},{fee / 1e8:.8f},{rate},{description},{kind}"
        )
        written += 1

    i = 0
    while written < count:
        i += 1
        t += datetime.timedelta(minutes=rng.randint(30, 300))
        rate = 30000 + rng.randint(0, 40000)
        r = rng.random()
        hot, cold = rng.choice([("Hot", "Cold"), ("LHot", "LCold")])
        if r < 0.35 or held[hot] < 3_000_000:
            amount = rng.randint(100_000, 5_000_000)
            held[hot] += amount
            add(hot, f"buy-{i}", "inbound", amount, 0, rate, "buy", "Buy")
        elif r < 0.55:
            amount = rng.randint(10_000, held[hot] // 4)
            held[hot] -= amount
            add(hot, f"sell-{i}", "outbound", amount, 0, rate, "sell", "Sell")
        elif r < 0.80 and hot == "Hot":
            # A same-txid transfer between the two Bitcoin wallets.
            amount = rng.randint(10_000, held[hot] // 3)
            fee = rng.randint(200, 5000)
            held[hot] -= amount + fee
            held[cold] += amount
            txid = f"{i:064x}"
            add(hot, txid, "outbound", amount + fee, fee, rate, "transfer", "To cold")
            add(cold, txid, "inbound", amount, 0, rate, "transfer", "From hot")
        elif r < 0.90 and held["Hot"] > 3_000_000:
            amount = rng.randint(10_000, held["Hot"] // 4)
            fee = rng.randint(200, 5000)
            held["Hot"] -= amount + fee
            held["LHot"] += amount
            add("Hot", f"pegout-{i}", "outbound", amount + fee, fee, rate, "transfer", "Peg in")
            add("LHot", f"pegin-{i}", "inbound", amount, 0, rate, "transfer", "Peg in")
            pegs.append((f"pegout-{i}", f"pegin-{i}"))
        elif held[cold] > 1_000_000:
            amount = rng.randint(10_000, held[cold] // 4)
            held[cold] -= amount
            add(cold, f"spend-{i}", "outbound", amount, 0, rate, "expense", "Spend")
    return rows, pegs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--kind", choices=sorted(SEEDS), required=True)
    parser.add_argument("--transactions", type=int, required=True)
    parser.add_argument("--seed", type=int, help="defaults to the shape's committed seed")
    parser.add_argument("--out", required=True)
    parser.add_argument("--force", action="store_true", help="replace an existing --out")
    args = parser.parse_args()
    seed = SEEDS[args.kind] if args.seed is None else args.seed
    out = Path(args.out).resolve()
    if out.exists():
        if not args.force:
            parser.error(f"{out} exists; pass --force to replace it")
        shutil.rmtree(out)
    out.mkdir(parents=True)
    root = out / "data"
    env = {**os.environ, "KASSIBER_DISABLE_UPDATE_CHECK": "1", "KASSIBER_NO_EGRESS": "1"}

    def run(*cli_args: str, check: bool = True) -> subprocess.CompletedProcess:
        result = cli_runner.run(
            ["--machine", "--data-root", str(root), *cli_args],
            reference=True, cwd=REPO_ROOT, env=env, capture_output=True, text=True,
        )
        if check and result.returncode != 0:
            raise SystemExit(f"FAILED {' '.join(cli_args)}: {result.stdout[:600]} {result.stderr[-600:]}")
        return result

    if args.kind == "generic":
        rows, pegs = generic_rows(args.transactions, seed), []
        wallets = {"W1": []}
        files = {"W1": "book.csv"}
    else:
        rows, pegs = multi_wallet_rows(args.transactions, seed)
        wallets = MULTI_WALLETS
        files = {wallet: f"{wallet}.csv" for wallet in wallets}
    digests = {}
    for wallet, lines in rows.items():
        text = HEADER + "\n" + "\n".join(lines) + "\n"
        (out / files[wallet]).write_text(text)
        digests[files[wallet]] = hashlib.sha256(text.encode()).hexdigest()

    started = time.perf_counter()
    run("init")
    run("workspaces", "create", "Main")
    run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", "generic", "Default")
    for wallet, extra in wallets.items():
        run("wallets", "create", *SCOPE, "--label", wallet, "--kind", "custom", *extra)
    for wallet in wallets:
        run("wallets", "import-csv", *SCOPE, "--wallet", wallet, "--file", str(out / files[wallet]))
    paired = 0
    for outbound, inbound in pegs[: min(MAX_PAIRED_PEGS, max(1, len(pegs) // 2))]:
        result = run("transfers", "pair", *SCOPE, "--tx-out", outbound, "--tx-in", inbound, "--kind", "peg-in", check=False)
        paired += result.returncode == 0
    processed = json.loads(run("journals", "process", *SCOPE).stdout)["data"]
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, capture_output=True, text=True).stdout.strip()
    manifest = {
        "schema": "kassiber.journals-bench-book.v1",
        "kind": args.kind,
        "transactions": sum(len(lines) for lines in rows.values()),
        "seed": seed,
        "csv_sha256": digests,
        "pegs": len(pegs),
        "pegs_paired": paired,
        "journals": {key: processed.get(key) for key in ("entries_created", "quarantined", "cross_asset_pairs", "processed_transactions")},
        "built_by": commit,
        "build_seconds": round(time.perf_counter() - started, 1),
    }
    (out / "bench-book.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
