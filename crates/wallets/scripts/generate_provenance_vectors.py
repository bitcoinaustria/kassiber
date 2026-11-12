#!/usr/bin/env python3
"""Unchanged Python provenance persistence and journal-input comparison oracle."""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from kassiber.core import imports, sync
from kassiber.core.custody_journal import projection_freshness
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION
from kassiber.core.chain_observer import provenance
from generate_insert_vectors import SEED, insert, observe, record, txid
from publication_books import deterministic, dump, error, new_book, snapshot

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_provenance.json"


def inputs(conn, entries):
    states, bindings = sync._observed_accounting_inputs(conn, "p", entries)
    return {"states": sorted([list(key), value] for key, value in states.items()),
            "bindings": sorted([key, list(value)] for key, value in bindings.items())}


def entries_for(records, observer_id="observer-2", kind="bdk"):
    identity = type("Identity", (), {"id": observer_id, "observer_kind": kind})()
    return provenance.provenance_entries_for_facts(((identity, records),), records)


def scenarios():
    receive = record(1, receive=90_000)
    send = record(2, spend=100_000, change=9_000)
    mempool = record(3, receive=7_000, confirmed=False)
    implicit = dict(record(4, receive=5_000), raw_json=json.dumps({"component": {"fee_attribution": "implicit_wallet_delta"}, "txid": txid(4)}))
    opaque = dict(record(5, receive=6_000), raw_json="not json")
    return [
        {"name": "fresh", "setup": [], "records": [receive, send, mempool, implicit, opaque]},
        {"name": "refresh-same", "setup": [("observe", [receive, send])], "records": [receive, send]},
        {"name": "refresh-identical", "setup": [("observe", [receive, send])], "records": [receive, send], "observer": "observer-1"},
        {"name": "confirmation-flip", "setup": [("observe", [mempool])], "records": [record(3, receive=7_000)]},
        {"name": "other-wallet-older-evidence", "setup": [("observe_b", [mempool]), ("observe", [mempool])],
         "records": [record(3, receive=7_000)]},
        {"name": "promote-kind", "setup": [("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,raw_json,created_at) VALUES('legacy','w','p','a','" + txid(1) + "','fp','2023-11-14T22:13:20Z','inbound','BTC',90000000,0,'{}','0000')")],
         "records": [receive], "skip_insert": True, "resolved": [{"transaction_id": "legacy", "external_id": txid(1), "asset": "btc", "direction": "INBOUND"}]},
        {"name": "duplicate-key", "setup": [], "records": [receive], "entries_extra": "duplicate"},
        {"name": "missing-resolution", "setup": [], "records": [receive], "resolved": []},
        {"name": "conflicting-resolution", "setup": [], "records": [receive], "resolved_extra": "conflict"},
        {"name": "invalid-resolution", "setup": [], "records": [receive], "resolved": [{"transaction_id": "", "txid": txid(1), "asset": "BTC", "direction": "inbound"}]},
        {"name": "row-missing", "setup": [], "records": [receive], "skip_insert": True, "resolved": [{"transaction_id": "nope", "external_id": txid(1), "asset": "BTC", "direction": "inbound"}]},
        {"name": "identity-mismatch", "setup": [("observe", [send])], "records": [receive], "skip_insert": True, "resolved_from": txid(2)},
        {"name": "non-native-chain", "setup": [("sql", "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES('ln','w','p','a','" + txid(1) + "','fp','2023-11-14T22:13:20Z','inbound','BTC',90000000,0,'0000')")],
         "records": [receive], "skip_insert": True, "chain": "lightning", "resolved": [{"transaction_id": "ln", "external_id": txid(1), "asset": "BTC", "direction": "inbound"}]},
        {"name": "processed-journal", "setup": [("observe", [receive]), ("sql", "UPDATE profiles SET last_processed_at='x',last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),last_processed_input_version=journal_input_version WHERE id='p'; UPDATE profiles SET journal_semantics_version=" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + " WHERE id='p'")],
         "records": [receive]},
    ]


def run(case):
    conn = new_book()
    conn.executescript(SEED)
    with deterministic(imports, provenance, prefix="setup"):
        for kind, payload in case["setup"]:
            if kind == "sql":
                conn.executescript(payload)
            else:
                observe(conn, payload, "b" if kind == "observe_b" else "a")
    conn.commit()
    with deterministic(imports, provenance) as counter:
        resolved = []
        if not case.get("skip_insert"):
            resolved = insert(conn, case["records"])["_observer_resolved_records"]
        conn.commit()
        if "resolved" in case:
            resolved = case["resolved"]
        if "resolved_from" in case:
            row = conn.execute("SELECT id FROM transactions WHERE external_id=?", (case["resolved_from"],)).fetchone()
            resolved = [{"transaction_id": row[0], "external_id": txid(1), "asset": "BTC", "direction": "inbound"}]
        if case.get("resolved_extra") == "conflict":
            resolved = [*resolved, dict(resolved[0], transaction_id="other")]
        entries = entries_for(case["records"], case.get("observer", "observer-2"))
        if case.get("entries_extra") == "duplicate":
            entries = [*entries, dict(entries[0], external_id=entries[0]["external_id"].upper())]
        before_dump = dump(conn)
        before_inputs = inputs(conn, entries)
        ids_before = counter["id"], counter["now"]
        conn.execute("SAVEPOINT step")
        try:
            outcome = {"result": provenance.persist_chain_observation_provenance(
                conn, conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone(),
                conn.execute("SELECT * FROM wallets WHERE id='a'").fetchone(), application_revision="revision",
                chain=case.get("chain", "bitcoin"), network="main", entries=entries, resolved_records=resolved)}
            conn.execute("RELEASE step")
        except Exception as exc:  # The vector records Python's refusal.
            conn.execute("ROLLBACK TO step")
            conn.execute("RELEASE step")
            outcome = {"error": error(exc)}
        return {"name": case["name"], "entries": entries, "resolved": resolved, "before": before_dump,
                "clock": list(ids_before), "inputs_before": before_inputs, "outcome": outcome,
                "inputs_after": inputs(conn, entries), "after": snapshot(conn),
                "journal_current": projection_freshness(conn, "p")["is_current"]}


def vectors():
    return {"generator": "crates/wallets/scripts/generate_provenance_vectors.py",
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
