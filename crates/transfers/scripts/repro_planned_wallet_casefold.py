#!/usr/bin/env python3
"""Reproduce a component plan that its own apply refuses.

Planned untracked-wallet ids use ``str.casefold()``, while the label lookup uses
SQLite's ASCII-only ``lower()``. After ``Straße`` exists, ``STRASSE`` is not
found by label but maps to the same planned id. The plan validates against the
existing wallet row and succeeds; apply then inserts that id again and reports
that ``STRASSE`` already exists. Uses a disposable book; no network.
"""
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import custody_component_planner as planner
from kassiber.db import open_db
from kassiber.errors import AppError


def spec(tx, label):
    return [{"legs": [
        {"role": "source", "transaction": tx, "amount_msat": 1000},
        {"role": "destination", "untracked_wallet": label, "amount_msat": 1000, "occurred_at": "2025-02-01T00:00:00Z"},
    ]}]


with tempfile.TemporaryDirectory(prefix="kassiber-casefold-repro-") as root:
    c = open_db(root)
    for q in (
        "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z')",
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025-01-01T00:00:00Z')",
        "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('a','w','p','treasury','Treasury','asset','2025-01-01T00:00:00Z')",
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('hot','w','p','Hot','custom','2025-01-01T00:00:00Z')",
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES('t1','w','p','hot','f1','BTC','outbound',1000,0,'2025-02-01T00:00:00Z','withdrawal','2025-01-01T00:00:00Z')",
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES('t2','w','p','hot','f2','BTC','outbound',1000,0,'2025-02-01T00:00:00Z','withdrawal','2025-01-01T00:00:00Z')",
    ):
        c.execute(q)
    c.commit()
    common = dict(workspace_id="w", profile_id="p", action="create", authored_source="cli", activate=False)
    version = c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
    planner.apply_component_review(c, expected_input_version=version, components=spec("t1", "Straße"), **common)
    plan = planner.plan_component_review(c, components=spec("t2", "STRASSE"), **common)
    print("plan input_version:", plan["input_version"], "summary:", plan["summary"])
    try:
        planner.apply_component_review(c, expected_input_version=plan["input_version"], components=spec("t2", "STRASSE"), **common)
    except AppError as error:
        print("apply refused:", error.code, error)
        assert error.code == "conflict" and "STRASSE" in str(error)
    else:
        raise AssertionError("Reference behavior changed; review repro")
    labels = [row[0] for row in c.execute("SELECT label FROM wallets ORDER BY label")]
    print("wallet labels:", labels)
    assert "STRASSE" not in labels
