#!/usr/bin/env python3
"""Unchanged Python durable policy-coverage oracle (one writer, not full sync)."""
import argparse
import json
from pathlib import Path
import sys
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from kassiber.core import ownership_policy_epochs as epochs
from kassiber.core.chain_observer.contract import CoveragePoint
from generate_insert_vectors import SEED
from publication_books import deterministic, dump, error, new_book, snapshot

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_policy.json"
CONFIG = json.dumps({"descriptor": "wpkh(synthetic)", "change_descriptor": "", "gap_limit": 20, "addresses": [],
                     "script_types": ["p2wpkh"], "label": "not private", "chain": "Bitcoin", "network": "mainnet",
                     "samourai": {"role": "x"}, "xpub": None})


def identity(**overrides):
    value = {"source_wallet_id": "a", "source_key": "descriptor:default", "observer_kind": "bdk",
             "chain": "bitcoin", "network": "main", "branch_keys": ("receive", "change", "receive")}
    value.update(overrides)
    return SimpleNamespace(**value)


def point(branch, scanned, used):
    return CoveragePoint(branch_key=branch, scanned_to=scanned, highest_used=used, details={})


def scenarios():
    first = [point("receive", 5, 2), point("change", 3, None)]
    return [
        {"name": "fresh", "rounds": [(identity(), first)]},
        {"name": "private-scan-depth", "rounds": [(identity(), first)],
         "config": json.dumps({**json.loads(CONFIG), "ownership_scan_to_index": 37})},
        {"name": "refresh", "rounds": [(identity(), first), (identity(observer_kind="bdk-2", branch_keys=("change",)), [point("receive", 9, 7)])]},
        {"name": "second-source", "rounds": [(identity(), first), (identity(source_key="xpub:p2tr"), [point("receive", 1, None)])]},
        {"name": "config-scope", "rounds": [(identity(chain="", network=""), [])]},
        {"name": "bad-network", "rounds": [(identity(network="moon"), first)]},
        {"name": "bad-chain", "rounds": [(identity(chain="dogecoin"), first)]},
        {"name": "missing-wallet", "rounds": [(identity(source_wallet_id="missing"), first)]},
        {"name": "retired-then-active", "setup": "INSERT INTO wallet_policy_epochs(id,workspace_id,profile_id,wallet_id,chain,network,status,private_material_json,created_at,retired_at) VALUES('old','w','p','a','bitcoin','main','retired','{}','x','y')",
         "rounds": [(identity(), first)]},
    ]


def run(case):
    conn = new_book()
    conn.executescript(SEED)
    conn.execute("UPDATE wallets SET config_json=? WHERE id='a'", (case.get("config", CONFIG),))
    if case.get("setup"):
        conn.executescript(case["setup"])
    conn.commit()
    steps = []
    with deterministic(epochs):
        for ident, coverage in case["rounds"]:
            before = dump(conn)
            conn.execute("SAVEPOINT step")
            try:
                result = {"result": epochs.record_observer_policy_coverage(conn, ident, coverage)}
                conn.execute("RELEASE step")
            except Exception as exc:  # The vector records Python's refusal.
                conn.execute("ROLLBACK TO step")
                conn.execute("RELEASE step")
                result = {"error": error(exc)}
            steps.append({"identity": {**vars(ident), "branch_keys": list(ident.branch_keys)},
                          "coverage": [[p.branch_key, p.scanned_to, p.highest_used] for p in coverage],
                          "before": before, "outcome": result, "after": snapshot(conn)})
    return {"name": case["name"], "steps": steps}


def vectors():
    return {"generator": "crates/wallets/scripts/generate_policy_vectors.py",
            "cases": [run(case) for case in scenarios()]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), indent=1, sort_keys=True, ensure_ascii=False, default=str) + "\n"
    if args.check:
        if DEST.read_text(encoding="utf-8") != text:
            raise SystemExit(f"{DEST} is stale")
        return
    DEST.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
