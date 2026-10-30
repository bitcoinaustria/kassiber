#!/usr/bin/env python3
"""Python's prepared fetches and full-book publication results, replayed offline by Rust.

Each case is one sync round against the loopback chain in `tests/rust_sync_chain.py`:
the book before publication, the serialized prepared fetch, and the outcome plus
every table afterwards under deterministic id/clock sequences. BDK's request
clock is frozen so first/last-seen state is reproducible.
"""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))
os.environ.setdefault("KASSIBER_OFFLINE_PREFERENCE_FILE", str(Path(tempfile.mkdtemp()) / "offline.json"))

import bdkpython as bdk  # noqa: E402

from tests.rust_sync_chain import serve  # noqa: E402
from tests.rust_sync_parity import (  # noqa: E402
    connect, copy_book, create_book, deserialize_fetch, dump, python_apply, python_prefetch, runtime, serialize_fetch,
    seed_reviewed_state,
)
from tests.test_rust_wallet_publication import _fixture, _sequence  # noqa: E402
from publication_books import snapshot  # noqa: E402

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_wallet_publication.json"
FIXTURES = ["empty", "receive-change", "checkpoint", "multi-script", "fixed", "gap-one", "reorg", "retraction",
            "mempool", "coinbase"]
PORTS = {"electrum": 47_181, "esplora": 47_182}
SEQUENCES = ["reanchor", "reorg-drop", "preflight-race", "mempool-drop-return", "replacement", "height-change",
             "force-full", "force-without-state"]


def frozen_bdk(clock):
    stack = ExitStack()
    stack.enter_context(patch.object(bdk.Wallet, "start_full_scan", lambda w: w.start_full_scan_at(clock)))
    stack.enter_context(patch.object(bdk.Wallet, "start_sync_with_revealed_spks",
                                     lambda w: w.start_sync_with_revealed_spks_at(clock)))
    return stack


def run(name, protocol, config, chain, rounds, state=None, *, reviewed=False, clear_authority=False):
    cases = []
    with tempfile.TemporaryDirectory() as tmp:
        book = Path(tmp) / "book.sqlite"
        create_book(book, config)
        with serve(chain, protocol, PORTS[protocol]) as url:
            rt = runtime(protocol, url)
            for index, mutate in enumerate(rounds):
                if mutate is not None:
                    mutate()
                force = bool(state and state.get("force"))
                sql = state.get("sql") if state else None
                conn = connect(book)
                if sql:
                    conn.executescript(sql)
                    conn.commit()
                if reviewed and index:
                    seed_reviewed_state(conn, clear_authority=clear_authority and index == 1)
                before = {table: {"columns": [d[0] for d in conn.execute(f'SELECT * FROM "{table}"').description],
                                  "rows": [list(row) for row in conn.execute(f'SELECT * FROM "{table}"')]}
                          for table, rows in snapshot(conn).items() if rows}
                with frozen_bdk(1_700_100_000 + index):
                    fetch, checkpoints = python_prefetch(conn, rt, force_full=force)
                data = serialize_fetch(fetch)
                if "error" in data:
                    conn.close()
                    cases.append({"name": f"{name}/{protocol}/{index}", "prepare_error": data["error"]})
                    continue
                captured = conn.execute("SELECT config_json FROM wallets WHERE id='a'").fetchone()[0]
                outcome = python_apply(conn, rt, deserialize_fetch(data), checkpoints, force_full=force,
                                       prefix=f"r{index}", now=index * 1000)
                for result in outcome.get("result", []):
                    result.pop("elapsed_ms", None)
                after = json.loads(json.dumps(snapshot(conn), sort_keys=True))
                conn.close()
                cases.append({"name": f"{name}/{protocol}/{index}", "before": before, "fetch": data,
                              "captured_config": captured, "prefix": f"r{index}", "now": index * 1000,
                              "outcome": outcome, "after": after})
    return cases


def vectors():
    cases = []
    for protocol in ("electrum", "esplora"):
        for fixture in FIXTURES:
            cases += run(fixture, protocol, *_fixture(fixture))
        for sequence in SEQUENCES:
            config, chain, rounds, state = _sequence(sequence)
            cases += run(sequence, protocol, config, chain, rounds, state)
        for fixture in ("checkpoint", "reorg", "retraction", "mempool"):
            cases += run(f"reviewed-{fixture}", protocol, *_fixture(fixture), reviewed=True)
        cases += run("reviewed-authority", protocol, *_fixture("checkpoint"), reviewed=True, clear_authority=True)
    return {"generator": "crates/wallets/scripts/generate_wallet_publication_vectors.py", "cases": cases}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n"
    if args.check:
        if DEST.read_text(encoding="utf-8") != text:
            raise SystemExit(f"{DEST} is stale")
        return
    DEST.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
