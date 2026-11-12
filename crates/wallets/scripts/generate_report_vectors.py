#!/usr/bin/env python3
"""Unchanged Python saved-report impact oracle (one writer, not full sync)."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from kassiber.core import filed_report_chain as chain, imports
from kassiber.core.chain_observer import provenance
from generate_insert_vectors import NETWORK, SEED, observe, record, txid
from publication_books import deterministic, dump, error, new_book, snapshot

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_reports.json"
SNAPSHOT = ("INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,"
            "period_end_year,content_sha256,created_at) VALUES('s1','w','p','summary.pdf','filed',2023,2023,'" + "0" * 64 + "','fixed')")
BOUND = ("INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,domains_json,"
         "acknowledgements_json,revision,inventory_digest,created_at) VALUES('p','env','{env}',NULL,'{domains}','[]',1,'digest','fixed')")


def depend(conn, n, *, snapshot="s1", tamper=None):
    row = conn.execute("SELECT id FROM transactions WHERE external_id=? AND wallet_id='a' ORDER BY created_at LIMIT 1", (txid(n),)).fetchone()
    dependency = {"snapshot_id": snapshot, "profile_id": "p", "transaction_id": row[0], "wallet_id": "a",
                  "chain": "bitcoin", "network": "main", "txid": txid(n)}
    observed = chain._current(conn, dependency)
    if tamper:
        observed = {**observed, **tamper}
    conn.execute("INSERT INTO filed_report_chain_dependencies(snapshot_id,profile_id,transaction_id,wallet_id,chain,network,txid,observation_json) VALUES(?,?,?,?,?,?,?,?)",
                 (*dependency.values(), chain._json(observed)))


def scenarios():
    receive = record(1, receive=90_000, extra=NETWORK)
    mempool = record(3, receive=7_000, confirmed=False)
    return [
        {"name": "unchanged", "observe": [[receive]], "depend": [(1, None)], "after": [[receive]]},
        {"name": "evidence-changed", "observe": [[receive]], "depend": [(1, None)],
         "after": [[record(1, receive=90_000, block_hash=txid(77), extra=NETWORK)]]},
        {"name": "threshold-reversed", "observe": [[receive]], "depend": [(1, None)],
         "after": [[record(1, receive=90_000, confirmed=False, extra=NETWORK)]]},
        {"name": "coverage-lost", "observe": [[receive]], "depend": [(1, None)], "sql": "DELETE FROM transactions"},
        {"name": "retracted", "observe": [[receive]], "depend": [(1, None)], "sql": "DELETE FROM transactions", "retracted": [txid(1).upper()]},
        {"name": "retracted-then-missing", "observe": [[receive]], "depend": [(1, None)], "sql": "DELETE FROM transactions",
         "retracted": [txid(1)], "second": []},
        {"name": "excluded", "observe": [[receive]], "depend": [(1, None)], "sql": "UPDATE transactions SET excluded=1"},
        {"name": "stale-authority", "observe": [[receive]], "depend": [(1, None)], "sql": "UPDATE transactions SET raw_json='{}'"},
        {"name": "two-snapshots-one-row", "observe": [[receive, mempool]], "depend": [(1, None), (3, {"confirmed": True})],
         "extra_snapshot": True, "after": [[record(3, receive=7_000)]]},
        {"name": "domain-changed", "observe": [[receive]], "depend": [(1, None)],
         "sql": BOUND.format(env="test", domains="[]")},
        {"name": "bound-domain", "observe": [[receive]], "depend": [(1, {"domain_id": "old"})],
         "sql": BOUND.format(env="main", domains=json.dumps([{"chain": "bitcoin", "network": "main", "domain_id": "bound-domain"}]))},
        {"name": "instance-domain", "observe": [[record(1, receive=90_000, extra={**NETWORK, "chain_instance_id": "lab"})]], "depend": [(1, {"domain_id": "x"})]},
        {"name": "no-dependencies", "observe": [[receive]], "depend": []},
        {"name": "latest-inbox", "observe": [[receive]], "depend": [(1, None)],
         "after": [[record(1, receive=90_000, block_hash=txid(77), extra=NETWORK)]],
         "restore": [receive]},
    ]


def run(case):
    conn = new_book()
    conn.executescript(SEED)
    conn.executescript(SNAPSHOT)
    with deterministic(imports, provenance, prefix="setup"):
        for records in case["observe"]:
            observe(conn, records)
        if case.get("extra_snapshot"):
            conn.execute(SNAPSHOT.replace("'s1'", "'s2'"))
        for n, tamper in case["depend"]:
            depend(conn, n, tamper=tamper)
            if case.get("extra_snapshot"):
                depend(conn, n, snapshot="s2", tamper=tamper)
        for records in case.get("after", []):
            observe(conn, records)
        if case.get("sql"):
            conn.executescript(case["sql"])
    conn.commit()
    steps = []
    with deterministic(chain):
        for index, retracted in enumerate([case.get("retracted", []), *([case["second"]] if "second" in case else []),
                                          *([[], []] if "restore" in case else [])]):
            if index == 1 and "restore" in case:
                with deterministic(imports, provenance, prefix="restore"):
                    observe(conn, case["restore"])
                conn.commit()
            before = dump(conn)
            conn.execute("SAVEPOINT step")
            try:
                outcome = {"result": chain.record_wallet_changes(conn, "p", "a", retracted_txids=retracted)}
                conn.execute("RELEASE step")
            except Exception as exc:  # The vector records Python's refusal.
                conn.execute("ROLLBACK TO step")
                conn.execute("RELEASE step")
                outcome = {"error": error(exc)}
            steps.append({"retracted": retracted, "before": before, "outcome": outcome, "after": snapshot(conn)})
    return {"name": case["name"], "steps": steps}


def vectors():
    return {"generator": "crates/wallets/scripts/generate_report_vectors.py",
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
