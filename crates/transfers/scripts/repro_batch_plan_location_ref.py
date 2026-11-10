#!/usr/bin/env python3
"""Reproduce a create plan that echoes local ``location_ref`` values.

Materialized components and revision plans drop ``location_ref`` from their
renderer/AI-safe projections. ``public_component_batch_plan`` keeps it on newly
planned legs, so ``ui.transfers.components.plan`` returns it to the caller.
Uses a disposable book; no network.
"""
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import custody_component_planner as planner
from kassiber.db import open_db

with tempfile.TemporaryDirectory(prefix="kassiber-location-repro-") as root:
    c = open_db(root)
    for q in (
        "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z')",
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025-01-01T00:00:00Z')",
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('hot','w','p','Hot','custom','2025-01-01T00:00:00Z')",
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES('t1','w','p','hot','f1','BTC','outbound',1000,0,'2025-02-01T00:00:00Z','withdrawal','2025-01-01T00:00:00Z')",
    ):
        c.execute(q)
    c.commit()
    plan = planner.plan_component_review(
        c, workspace_id="w", profile_id="p", action="create", authored_source="gui", activate=False,
        components=[{"legs": [{"role": "source", "transaction": "t1", "amount_msat": 1000, "location_ref": "node:private"}]}],
    )
    exposed = [leg.get("location_ref") for leg in plan["components"][0]["legs"]]
    print("planned leg location_ref values:", exposed)
    assert exposed == ["node:private"], "Reference behavior changed; review repro"
