#!/usr/bin/env python3
"""Freeze local desktop responses and stored setup state from Python."""
import argparse
import copy
import itertools
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
from unittest import mock
import uuid
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber import daemon
from kassiber.core import accounts, maintenance
from kassiber.core.ui_snapshot import build_profiles_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError

DESTINATION = Path(__file__).resolve().parents[1] / "fixtures/python_desktop.json"
TABLES = ("workspaces", "profiles", "accounts", "wallets", "transactions", "custody_components", "custody_component_legs", "custody_gap_reviews", "custody_gap_review_transactions", "custody_component_purge_authorizations", "saved_views", "attachments", "ai_chat_sessions", "ai_chat_messages", "rates_cache", "rates_checked_minutes", "loan_legs", "book_reset_transactions", "book_reset_parked_rows")

class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 1, 1, tzinfo=timezone.utc).astimezone(tz)

def generate():
    with tempfile.TemporaryDirectory(prefix="kassiber-desktop-vectors-") as directory:
        conn = open_db(Path(directory) / "data")
        serial = itertools.count(1)
        records = []
        def record(kind, args=None, sql=""):
            args = args or {}
            if sql:
                conn.executescript(sql)
            try:
                if kind == "ui.profiles.snapshot":
                    result = build_profiles_snapshot(conn)
                elif kind == "ui.workspace.delete":
                    result = daemon._delete_current_workspace(SimpleNamespace(conn=conn))
                elif kind == "ui.profiles.reset_data":
                    result = maintenance.reset_current_profile_data(conn, str(Path(directory) / "data"), clear_shared_rates=args.get("clear_shared_rates", False), wipe_everything=args.get("wipe_everything", False))
                else:
                    handler = {"ui.workspace.create": daemon._create_workspace_payload, "ui.workspace.rename": daemon._rename_workspace_payload, "ui.profiles.create": daemon._create_profile_payload, "ui.profiles.switch": daemon._switch_profile_payload}[kind]
                    result = handler(conn, args)
                outcome = {"ok": result}
            except AppError as error:
                conn.rollback()
                outcome = {"error": {"code": error.code, "message": str(error), "hint": error.hint, "details": error.details, "retryable": error.retryable}}
            state = {table: [dict(r) for r in conn.execute(f"SELECT * FROM {table} ORDER BY rowid")] for table in TABLES}
            state["context"] = [dict(r) for r in conn.execute("SELECT * FROM settings WHERE key IN ('context_workspace','context_profile') ORDER BY key")]
            records.append(copy.deepcopy({"kind": kind, "args": args, "sql": sql, **outcome, "state": state}))
            return outcome.get("ok")
        with mock.patch("kassiber.core.accounts.uuid.uuid4", side_effect=lambda: uuid.UUID(f"00000000-0000-4000-8000-{next(serial):012x}")), mock.patch("kassiber.core.accounts.now_iso", return_value="2026-01-01T00:00:00Z"), mock.patch("kassiber.core.maintenance.now_iso", return_value="2026-01-01T00:00:00Z"), mock.patch("kassiber.core.ui_snapshot.datetime", FixedDateTime):
            record("ui.profiles.snapshot")
            for label in (None, 9, " "):
                record("ui.workspace.create", {"label": label})
            w = record("ui.workspace.create", {"label": " Main "})["workspace"]["id"]
            record("ui.workspace.create", {"label": "Main"})
            for args in ({}, {"workspace_id": w}, {"workspace_id": "Main", "label": "Wrong"}):
                record("ui.workspace.rename", args)
            p = record("ui.profiles.create", {"workspace_id": w, "label": "Austrian FIFO", "tax_country": "at", "gains_algorithm": "FIFO"})["profile"]["id"]
            record("ui.profiles.create", {"workspace_id": w, "label": "Generic", "tax_country": "generic", "fiat_currency": "CHF"})
            record("ui.profiles.create", {"workspace_id": w, "label": "Austrian ATM", "tax_country": "at"})
            record("ui.profiles.snapshot")
            other = record("ui.workspace.create", {"label": "Other"})["workspace"]["id"]
            record("ui.workspace.rename", {"workspace_id": other, "label": "Main"})
            record("ui.workspace.rename", {"workspace_id": w, "label": "Renamed"})
            record("ui.profiles.snapshot")
            record("ui.profiles.switch", {"profile_id": p})
            record("ui.profiles.snapshot")
            seed = f"""
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('w','{w}','{p}','Cold','address','t');
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,direction,asset,amount,fee,occurred_at,created_at) VALUES('tx','{w}','{p}','w','tx','tx','inbound','BTC',1000,0,'2026-01-01T00:00:00Z','t');
INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,expected_leg_count,expected_allocation_count,created_at) VALUES('component','lineage','{w}','{p}',1,'transfer',1,0,'t');
INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,ordinal,role,rail,asset,exposure,conservation_unit,amount_msat,created_at) VALUES('leg','component','{w}','{p}',0,'source','bitcoin','BTC','asset','BTC',1000,'t');
INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,authored_source,snapshot_json,created_at) VALUES('review','{w}','{p}','gap',1,'{'a'*64}','dismissed','user','{{}}','t');
INSERT INTO custody_gap_review_transactions(id,review_id,workspace_id,profile_id,role,transaction_id,created_at) VALUES('review-tx','review','{w}','{p}','source','tx','t');
INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','2026-01-01T00:00:00Z','50000','manual','t');
INSERT INTO loan_legs(id,workspace_id,profile_id,transaction_id,loan_id,role,created_at) VALUES('loan','{w}','{p}','tx','loan-1','collateral','t');
UPDATE transactions SET note='Kept note', fiat_rate=51000.5, pricing_source_kind='manual_override' WHERE id='tx';
"""
            record("ui.profiles.reset_data", sql=seed)
            record("ui.profiles.snapshot")
            record("ui.profiles.reset_data", {"clear_shared_rates": True})
            record("ui.profiles.reset_data", {"wipe_everything": True})
            record("ui.workspace.delete")
            record("ui.profiles.snapshot")
        conn.close()
        return {"tables": TABLES, "records": records}

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(generate(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        if not DESTINATION.exists() or DESTINATION.read_text() != encoded:
            raise SystemExit("desktop vectors differ; rerun export_desktop_vectors.py")
    else:
        DESTINATION.write_text(encoded)
