#!/usr/bin/env python3
"""Unchanged Python output-inventory oracle (one writer, not full sync)."""
import argparse
import json
from pathlib import Path
import sys
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from kassiber.core import output_inventory
from generate_insert_vectors import SEED, txid
from publication_books import deterministic, dump, error, new_book, snapshot

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_inventory.json"
SAMOURAI = json.dumps({"chain": "bitcoin", "samourai": {"role": "postmix", "mix_count": "3", "whirlpool": 1,
    "round_txids": [txid(9).upper(), "bad"], "sections": [" Premix ", ""], "tx0_role": "PREMIX", "exit_kind": "nope",
    "group_label": " " + "x" * 140, "watch_only": 0, "gap_limit": -1, "unknown": "dropped", "paynym": None,
    "source": None, "bip47": {"nested": True}}})


def output(n, vout=0, **extra):
    value = {"txid": txid(n), "vout": vout, "asset": "BTC", "amount_sats": 1000 + n, "confirmation_status": "confirmed",
             "confirmations": 3, "block_height": 5, "block_time": 1_700_000_000, "chain": "bitcoin", "network": "main",
             "branch_index": 0, "branch_label": "receive", "address_index": n, "address": f"bc1q{n}",
             "script_pubkey": "0014" + f"{n:02x}" * 20, "derivation_path": None, "derivation_paths": [], "key_origins": [],
             "raw": {"source": "bdk_wallet", "confirmed": True}}
    value.update(extra)
    return value


def scenarios():
    first = [output(1), output(2), output(3, vout=1, block_height=None, confirmation_status=None, raw=None)]
    return [
        {"name": "fresh", "rounds": [first]},
        {"name": "spend-and-keep", "rounds": [first, [output(1), output(4)]]},
        {"name": "all-spent", "rounds": [first, []]},
        {"name": "other-backend-untouched", "rounds": [first, [output(5)]], "backends": [("one", "electrum"), ("two", "esplora")]},
        {"name": "same-kind-other-name-untouched", "rounds": [first, []],
         "backends": [("one", "electrum"), ("two", "electrum")]},
        {"name": "flags-and-aliases", "rounds": [[output(6, spent=True), output(7, spentBy=txid(8), anonymityScore="4", excludedFromCoinjoin="yes",
            anonHistory=[{"a": 1}], keyState="used", asset="l-btc", chain=None, network="", script_pubkey="ABCD", block_height=0),
            output(8, spent_at="2020", spent="maybe", excluded_from_coinjoin="no", anon_history="bad", asset=None, address="  ",
                   script_pubkey="abc", confirmations=None, branch_index="", address_index="2")]]},
        {"name": "samourai", "rounds": [[output(1, raw={"samourai": {"mix_count": 9, "role": "x"}, "z": 1}), output(2, raw="text")]], "config": SAMOURAI},
        {"name": "bad-txid", "rounds": [[output(1), output(2, txid="nothex")]]},
        {"name": "missing-txid", "rounds": [[output(1, txid=None)]]},
        {"name": "negative-vout", "rounds": [[output(1, vout=-1)]]},
        {"name": "missing-vout", "rounds": [[output(1, vout="")]]},
        {"name": "missing-amount", "rounds": [[output(1, amount_sats=None)]]},
        {"name": "bad-confirmations", "rounds": [[output(1, confirmations=-2)]]},
        {"name": "text-height", "rounds": [[output(1, block_height="x")]]},
    ]


def run(case):
    conn = new_book()
    conn.executescript(SEED)
    if case.get("config"):
        conn.execute("UPDATE wallets SET config_json=? WHERE id='a'", (case["config"],))
    conn.commit()
    steps = []
    backends = case.get("backends", [("fixture", "electrum")] * len(case["rounds"]))
    with deterministic(output_inventory):
        for outputs, (name, kind) in zip(case["rounds"], backends):
            before = dump(conn)
            conn.execute("SAVEPOINT step")
            try:
                result = {"result": output_inventory.update_wallet_output_inventory(
                    conn, conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone(),
                    conn.execute("SELECT * FROM wallets WHERE id='a'").fetchone(), {"name": name, "kind": kind},
                    SimpleNamespace(chain="bitcoin", network="main"), outputs, commit=False)}
                conn.execute("RELEASE step")
            except Exception as exc:  # The vector records Python's refusal.
                conn.execute("ROLLBACK TO step")
                conn.execute("RELEASE step")
                result = {"error": error(exc)}
            steps.append({"outputs": outputs, "backend": [name, kind], "before": before, "outcome": result, "after": snapshot(conn)})
    return {"name": case["name"], "steps": steps}


def vectors():
    return {"generator": "crates/wallets/scripts/generate_inventory_vectors.py",
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
