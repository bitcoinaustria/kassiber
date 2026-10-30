#!/usr/bin/env python3
"""Unchanged Python retraction/final-metadata oracle; not full sync acceptance."""
import argparse
import json
from pathlib import Path
import sqlite3
import sys
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber.core import freshness, imports, repo
from kassiber.cli import handlers

DEST = Path(__file__).resolve().parents[1] / "tests/vectors/python_publication.json"
SEED = """
INSERT INTO workspaces(id,label,created_at) VALUES('w','synthetic','fixed');
INSERT INTO profiles(id,workspace_id,label,created_at,last_processed_at,last_processed_tx_count,journal_input_version,last_processed_input_version,ownership_review_counts_json)
VALUES('p','w','primary','fixed','processed',8,7,7,'{"kept":true}'),('q','w','other','fixed','processed',1,3,3,'{}');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
VALUES('a','w','p','Wällét','descriptor','{"z":"é","descriptor":"synthetic","nested":{"b":2,"a":1}}','fixed'),
('b','w','p','other-wallet','descriptor','{}','fixed'),('c','w','q','other-profile','descriptor','{}','fixed');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at)
VALUES('one','w','p','a','ABC','fp-one','date-one','in','BTC',9223372036854775807,1000,'fixed'),
('two','w','p','a','def','fp-two','date-two','out','BTC',1001,1,'fixed'),
('idless','w','p','a',NULL,'fp-idless','date-three','in','BTC',1000,0,'fixed'),
('other-wallet','w','p','b','ABC','fp-four','date-four','in','BTC',1000,0,'fixed'),
('other-profile','w','q','c','ABC','fp-five','date-five','in','BTC',1000,0,'fixed');
INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('tag','w','p','retained-tag','retained tag','fixed');
INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('one','tag'),('two','tag');
"""


def snapshot(conn):
    return {table: sorted([list(row) for row in conn.execute(f'SELECT * FROM "{table}"')],
                           key=lambda row: json.dumps(row, sort_keys=True))
            for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")}


def vectors():
    cases = [
        {"name": "empty", "ids": [], "checkpoint": {}},
        {"name": "missing", "ids": ["missing", "", "\u001c  "], "checkpoint": None},
        {"name": "delete-one", "ids": ["\u001c ABC ", "\u001c aBc ", ""], "checkpoint": {"z": "é", "tip": {"height": 10}}},
        {"name": "delete-both", "ids": ["def", "ABC"], "checkpoint": {"tip": None}},
        {"name": "partial", "ids": ["def"], "checkpoint": {}, "partial_success": True, "degraded_reason": "synthetic"},
        {"name": "blocking", "ids": [], "checkpoint": {}, "blocking_reports": True},
        {"name": "paused", "ids": ["abc"], "checkpoint": {}, "paused": True, "blocking_reports": True},
        {"name": "resume", "ids": [], "checkpoint": {"highest_used": {"receive": 0}}, "prior": True},
    ]
    result = []
    for case in cases:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA foreign_keys=ON")
        manifest = json.loads((Path(__file__).resolve().parents[2] / "store/fixtures/current-schema.json").read_text())
        for kind in ("table", "index", "view", "trigger"):
            for obj in manifest["objects"]:
                if obj[0] == kind and obj[3]:
                    conn.execute(obj[3])
        conn.execute("INSERT INTO chain_index_clock VALUES(1,0)")
        conn.commit()
        conn.executescript(SEED)
        prior_sql = ""
        if case.get("paused") or case.get("prior"):
            with patch("kassiber.core.freshness.now_iso", return_value="old"):
                freshness.upsert_source_state(conn, profile_id="p", source_key="onchain_wallet:a", source_type="onchain_wallet",
                    source_label="old", status="error", paused=case.get("paused", False), retry_count=4,
                    rate_limited_until="2999-01-01T00:00:00+00:00", cooldown_reason="old",
                    last_error_at="old", last_error_code="old", last_error_message="old", checkpoint={"old": True})
            row = conn.execute("SELECT * FROM freshness_source_states").fetchone()
            columns = ",".join(row.keys())
            literals = ",".join("NULL" if value is None else str(value) if isinstance(value, int) else "'" + value.replace("'", "''") + "'" for value in row)
            prior_sql = f"INSERT INTO freshness_source_states({columns}) VALUES({literals});"
        conn.commit()
        profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id='a'").fetchone()
        conn.execute("SAVEPOINT wallet_refresh")
        outcome = imports.retract_wallet_records(conn, profile, wallet, case["ids"], "backend:synthetic",
            imports.ImportCoordinatorHooks(lambda *args: None, repo.invalidate_journals), commit=False)
        with patch("kassiber.cli.handlers.now_iso", return_value="now"), patch("kassiber.core.freshness.now_iso", return_value="updated-now"):
            handlers._mark_wallet_synced_from_results(conn, wallet, [{"status": "synced", "freshness_checkpoint": case["checkpoint"],
                "partial_success": case.get("partial_success", False), "silent_payment_degraded_reason": case.get("degraded_reason"),
                "blocking_reports": case.get("blocking_reports", False)}])
        conn.execute("RELEASE wallet_refresh")
        result.append({"input": case, "prior_sql": prior_sql, "outcome": outcome, "snapshot": snapshot(conn)})
        conn.close()
    return {"seed": SEED, "cases": result}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    output = json.dumps(vectors(), indent=2, ensure_ascii=False) + "\n"
    if args.check:
        if DEST.read_text() != output:
            raise SystemExit("Publication oracle drift")
    else:
        DEST.write_text(output)
