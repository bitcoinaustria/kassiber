#!/usr/bin/env python3
"""Freeze Python's workspace shell reads on synthetic disposable books.

Every case records health, next actions, badges, the active-book overview and
the book-set overview. Timestamps predate the generation date by design, so
relative labels, year-to-date sums and exit-tax cutoffs stay stable.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import ui_snapshot as ui
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db

DESTINATION = ROOT / "crates/workspace/tests/data/vectors.json"
BASE = (ROOT / "crates/transactions/tests/data/desktop.sql").read_text()
RELATION = "INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,created_at) VALUES('" + "a" * 64 + "','w','p','conversion','g','d','BTC','LBTC',100001,100001,'eligible','2025','2025');"
CURRENT = "UPDATE profiles SET last_processed_at='2026-01-01T00:00:00Z',last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id=profiles.id AND excluded=0),last_processed_input_version=journal_input_version; UPDATE profiles SET journal_semantics_version=" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + ";"


def quarantine(transaction, reason, detail):
    payload = json.dumps(detail).replace("'", "''")
    return f"INSERT OR REPLACE INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at) VALUES('{transaction}','w','p','{reason}','{payload}','2025');"


def cases():
    yield "no_scope", "empty", ""
    yield "no_scope", "workspace_only", "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025'); INSERT INTO settings(key,value) VALUES('context_workspace','w');"
    for name, sql in (
        ("unprocessed", ""),
        ("count_stale", CURRENT + "UPDATE profiles SET last_processed_tx_count=1 WHERE id='p';"),
        ("version_stale", CURRENT + "UPDATE profiles SET last_processed_input_version=journal_input_version+1 WHERE id='p';"),
        ("current", CURRENT),
        ("current_no_quarantine", CURRENT + "DELETE FROM journal_quarantines;"),
        ("current_all_prices", CURRENT + "DELETE FROM journal_quarantines; UPDATE transactions SET fiat_rate=2,fiat_rate_exact='2';"),
        ("cached_prices", CURRENT + "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2020',2,'2','manual','2025');"),
        ("missing_profile", "UPDATE settings SET value='absent' WHERE key='context_profile';"),
        ("wrong_workspace", "UPDATE settings SET value='absent' WHERE key='context_workspace';"),
        ("foreign_scope", "UPDATE settings SET value='foreign' WHERE key='context_profile';"),
        ("excluded", "UPDATE transactions SET excluded=1 WHERE profile_id='p';" + CURRENT),
        ("no_wallets", "DELETE FROM wallets WHERE profile_id='p';"),
        ("no_transactions", "DELETE FROM transactions WHERE profile_id='p';"),
        ("cached_zero", "UPDATE profiles SET swap_candidate_count=0 WHERE id='p';"),
        ("cached_nonzero", "UPDATE profiles SET swap_candidate_count=19 WHERE id='p';"),
        ("paired_current", CURRENT + RELATION),
        ("paired_stale", CURRENT + RELATION + "UPDATE profiles SET last_processed_input_version=journal_input_version+1 WHERE id='p';"),
        ("paired_urgency", CURRENT + RELATION + "UPDATE transactions SET review_status='failed' WHERE id='d'; UPDATE transactions SET review_status='pending' WHERE id='g';"),
        ("fractional_markers", CURRENT + "UPDATE profiles SET last_processed_tx_count=7.9,journal_input_version=10.9,last_processed_input_version=10.1,tax_long_term_days=400.9 WHERE id='p';"),
        ("wide_markers", CURRENT + "UPDATE profiles SET journal_input_version='999999999999999999999999999999999',last_processed_input_version='999999999999999999999999999999999' WHERE id='p';"),
        ("cached_fraction", "UPDATE profiles SET swap_candidate_count=4.9 WHERE id='p';"),
        ("cached_bad", "UPDATE profiles SET swap_candidate_count='invalid' WHERE id='p';"),
        ("semantics_required", CURRENT + "DELETE FROM journal_quarantines; UPDATE profiles SET required_journal_semantics_version=1,journal_semantics_version=0 WHERE id='p';"),
        ("semantics_built", CURRENT + "DELETE FROM journal_quarantines; UPDATE profiles SET required_journal_semantics_version=1,journal_semantics_version=" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + " WHERE id='p';"),
        ("semantics_old_build", CURRENT + "DELETE FROM journal_quarantines; UPDATE profiles SET required_journal_semantics_version=1,journal_semantics_version=1 WHERE id='p';"),
    ):
        yield "health", name, BASE + sql
    for name, tail in (
        ("named_root", quarantine("d", "custody_basis_barrier", {"root_transaction_ids": ["c"]})),
        ("unknown_root", quarantine("d", "custody_basis_barrier", {"root_transaction_ids": ["not-present"]})),
        ("dependency_chain", quarantine("d", "custody_basis_barrier", {"root_transaction_ids": ["c"]}) + quarantine("g", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["d"]})),
        ("cycle", quarantine("d", "custody_basis_barrier", {"root_transaction_ids": ["g"]}) + quarantine("g", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["d"]})),
        ("group_root", quarantine("c", "missing_fee_price", {"transfer_group_id": "group"}) + quarantine("d", "derived_transfer_group_blocked", {"transfer_group_id": "group"})),
        ("numeric_group", quarantine("c", "missing_fee_price", {"transfer_group_id": 21}) + quarantine("d", "derived_transfer_group_blocked", {"transfer_group_id": 21})),
        ("boolean_group", quarantine("c", "missing_fee_price", {"transfer_group_id": 1}) + quarantine("d", "derived_transfer_group_blocked", {"transfer_group_id": True})),
        ("non_scalar_group", quarantine("d", "derived_transfer_group_blocked", {"transfer_group_id": ["group"]})),
        ("basis_root", quarantine("e", "basis_provenance_incomplete", {"lot_state_uncertain_since": "2025-02-01T00:00:00Z"})),
        ("ambiguous_basis", quarantine("a", "missing_fee_price", {}) + quarantine("e", "basis_provenance_incomplete", {"lot_state_uncertain_since": "2025-02-01T00:00:00Z"})),
        ("different_asset", quarantine("d", "basis_provenance_incomplete", {"lot_state_uncertain_since": "2025-02-01T00:00:00Z"})),
        ("bad_json", "UPDATE journal_quarantines SET detail_json='not-json';" + quarantine("d", "at_swap_basis_carry_unresolved", {})),
        ("unknown_reason", quarantine("d", "new_reason", {})),
    ):
        yield "badges", name, BASE + CURRENT + tail


JOURNAL = (
    "INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,fiat_value,unit_cost,cost_basis,gain_loss,description,at_category,created_at) VALUES"
    "('j1','w','p','a','cold','2020-05-01T00:00:00Z','acquisition','BTC',300000000000,1200.5,1,NULL,NULL,'at_regime=alt',NULL,'2025'),"
    "('j2','w','p','b','ln','2025-02-01T00:00:00Z','acquisition','BTC',100001,3.25,1,NULL,NULL,'',NULL,'2025'),"
    "('j3','w','p','c','exchange','2025-02-01T00:00:00Z','disposal','BTC',-100001,0.1,1,0.07,0.03,'at_regime=neu','neu_gain','2025'),"
    "('j4','w','p','g','cold','2025-01-02T00:00:00Z','transfer_out','BTC',-50000000000,0,1,400.0,NULL,'at_alt_out=40000000000 at_regime=alt',NULL,'2025'),"
    "('j5','w','p','e','custom','2025-01-02T00:00:00Z','transfer_in','BTC',50000000000,400.0,1,NULL,NULL,'at_alt_in=40000000000',NULL,'2025'),"
    "('j6','w','p','g','cold','2025-01-02T00:00:00Z','transfer_fee','BTC',-3,0,1,0.0001,NULL,'',NULL,'2025'),"
    "('j7','w','p','d','liquid','2025-01-01T00:00:00Z','income','LBTC',1001,0.5,1,NULL,NULL,'',NULL,'2025'),"
    "('j8','w','p','d','liquid','2019-01-01T00:00:00Z','acquisition','LBTC',2002,0.75,1,NULL,NULL,'',NULL,'2025');"
)
INVENTORY = (
    "UPDATE wallets SET config_json='{\"descriptor\":\"wpkh(private)\",\"backend\":\"node\",\"gap_limit\":\" 7 \",\"last_synced_at\":\"2024-12-01T10:00:00Z\"}' WHERE id='cold';"
    "UPDATE wallets SET kind='descriptor',config_json='{\"descriptor\":\"wpkh(other)\",\"deprecated\":\"yes\",\"chain\":\"BTC\",\"network\":\"Testnet\"}' WHERE id='custom';"
    "INSERT INTO backends(name,kind,url,chain,network,created_at,updated_at) VALUES('node','esplora','http://127.0.0.1:1','bitcoin','regtest','2025','2025');"
    "INSERT INTO settings(key,value) VALUES('default_backend','node');"
    "INSERT INTO wallet_utxos(workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,anon_history_json,first_seen_at,last_seen_at,raw_json) VALUES"
    "('w','p','cold','node','esplora','bitcoin','regtest','BTC',70000000,'aa',0,'aa:0','confirmed','[]','2024','2024-12-02T00:00:00Z','{}'),"
    "('w','p','cold','node','esplora','bitcoin','regtest','btc',1234,'bb',1,'','confirmed','[]','2024','2024-12-03T00:00:00Z','{}'),"
    "('w','p','cold','stale','electrum','bitcoin','main','BTC',999,'cc',0,'cc:0','confirmed','[]','2024','2024-12-03T00:00:00Z','{}'),"
    "('w','p','custom','node','esplora','bitcoin','regtest','BTC',70000000,'aa',0,'aa:0','confirmed','[]','2024','2024-12-04T00:00:00Z','{}'),"
    "('w','p','custom','node','esplora','bitcoin','regtest','USDT',5,'dd',0,'dd:0','confirmed','[]','2024','2024-12-04T00:00:00Z','{}');"
    "INSERT INTO wallet_utxo_refreshes(wallet_id,workspace_id,profile_id,backend_name,backend_kind,chain,network,observed_count,active_count,last_seen_at) VALUES"
    "('cold','w','p','node','esplora','bitcoin','regtest',3,2,'2024-12-05T00:00:00Z');"
)
RATES = (
    "INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at,granularity,method) VALUES"
    "('BTC-EUR','2025-01-01T00:00:00Z',90000.5,'coinbase','2025-01-02','daily','candles'),"
    "('BTC-EUR','2025-01-01T12:00:00Z',91000,'manual','2025-01-02','minute','manual'),"
    "('BTC-EUR','2025-01-03T00:00:00Z',92000,'kraken-csv','2025-01-04','daily','ohlcvt_csv'),"
    "('BTC-EUR','2025-02-15T00:00:00Z',0,'coinbase','2025-02-16','daily','candles'),"
    "('BTC-USD','2025-01-01T00:00:00Z',100000,'coinbase','2025-01-02','daily','candles');"
)
EXTRA_BOOKS = (
    "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('usd','w','Dollars','usd','generic','2026'),('empty','w','Empty','EUR','generic','2027');"
    "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('usd-w','w','usd','Dollar wallet','coreln','{\"chain\":\"weird\"}','2024-01-01T00:00:00Z');"
    "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at,kind,fiat_rate,fiat_value) VALUES('u1','w','usd','usd-w','u1','BTC','inbound',250000000,0,'2024-06-01T00:00:00Z','2024','deposit',NULL,30.5);"
)
CANONICAL = (
    "INSERT INTO journal_quantity_postings(posting_id,workspace_id,profile_id,transaction_id,occurred_at,asset,location_kind,location_id,amount_msat,state,created_at) VALUES"
    "('q1','w','p','a','2025-02-01T00:00:00Z','BTC','wallet','cold',300000000000,'observed','2025'),"
    "('q2','w','p','c','2025-02-01T00:00:00Z','BTC','wallet','exchange',-100002,'observed','2025'),"
    "('q3','w','p','c','2025-02-01T00:00:00Z','BTC','external','x',100002,'external_presumed','2025');"
    "INSERT INTO journal_quantity_balances(workspace_id,profile_id,location_kind,location_id,asset,amount_msat,created_at) VALUES"
    "('w','p','wallet','cold','BTC',300000000000,'2025'),('w','p','wallet','exchange','BTC',-100002,'2025');"
)
ISSUES = (
    "INSERT INTO journal_quantity_issues(issue_id,workspace_id,profile_id,issue_type,state,asset,amount_msat,occurred_at,transaction_ids_json,reason,detail_json,blocks_from,created_at) VALUES"
    "('i1','w','p','gap','custody_candidate','BTC',5,'2025-01-05T00:00:00Z','[]','r','{}','2025-01-05T00:00:00Z','2025');"
)
HOLDINGS = "INSERT INTO journal_wallet_holdings(workspace_id,profile_id,wallet_id,wallet_label,account_code,asset,quantity,cost_basis,created_at) VALUES('w','p','cold','Cold','','BTC',250000000000,1000,'2025'),('w','p','ln','Lightning','','LBTC',7,1,'2025');"
PAIRS = RELATION + (
    "INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,fiat_value,unit_cost,cost_basis,description,created_at) VALUES"
    "('t1','w','p','c','exchange','2025-02-01T00:00:00Z','transfer_out','BTC',-100001,0,1,0.02,'Move','2025'),"
    "('t2','w','p','b','ln','2025-02-01T00:00:00Z','transfer_in','BTC',100001,0.02,1,NULL,'Move','2025');"
)
ALT_MOVED = (
    "INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,fiat_value,unit_cost,cost_basis,description,created_at) VALUES"
    "('j9','w','p','b','ln','2020-06-01T00:00:00Z','acquisition','BTC',5000,1,1,NULL,'at_regime=alt','2025'),"
    "('j10','w','p','b','ln','2025-03-01T00:00:00Z','transfer_out','BTC',-5000,0,1,0.1,'at_alt_out=5000 at_regime=alt','2025');"
)
IMPORTS = "INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,imported_at,rows_inserted,rows_updated,rows_skipped) VALUES('ib1','w','p','exchange','kraken_csv','2024-01-01T00:00:00Z',1,0,0),('ib2','w','p','exchange','kraken_ledger','2024-02-01T00:00:00Z',1,0,0);"


def overview_cases():
    for name, sql in (
        ("ov_unprocessed", ""),
        ("ov_journal", CURRENT + JOURNAL),
        ("ov_journal_rates", CURRENT + JOURNAL + RATES),
        ("ov_holdings", CURRENT + JOURNAL + HOLDINGS),
        ("ov_canonical", CURRENT + JOURNAL + CANONICAL),
        ("ov_canonical_issues", CURRENT + JOURNAL + CANONICAL + ISSUES + RATES),
        ("ov_inventory", INVENTORY),
        ("ov_inventory_current", CURRENT + JOURNAL + INVENTORY + RATES),
        ("ov_pairs", CURRENT + JOURNAL + PAIRS + IMPORTS),
        ("ov_books", EXTRA_BOOKS + RATES),
        ("ov_books_current", CURRENT + JOURNAL + EXTRA_BOOKS + PAIRS + INVENTORY),
        ("ov_generic", CURRENT + JOURNAL + RATES + "UPDATE profiles SET tax_country='generic',fiat_currency='' WHERE id='p';"),
        ("ov_issue_unknown_start", CURRENT + JOURNAL + CANONICAL + ISSUES.replace("'2025-01-05T00:00:00Z','2025');", "NULL,'2025');")),
        ("ov_alt_moved_out", CURRENT + JOURNAL + RATES + ALT_MOVED),
        ("ov_semantics_stale", CURRENT + JOURNAL + "UPDATE profiles SET required_journal_semantics_version=1,journal_semantics_version=1 WHERE id='p';"),
    ):
        yield "overview", name, BASE + sql


def workspace_overview(conn):
    try:
        return ui.build_workspace_overview_snapshot(conn, {"workspace_id": "w"})
    except Exception as error:  # the case has no workspace row
        return {"error": type(error).__name__, "message": str(error)}


def generate():
    output = []
    for group, name, sql in (*cases(), *overview_cases()):
        with tempfile.TemporaryDirectory(prefix="workspace-vectors-") as directory:
            conn = open_db(Path(directory) / "data")
            conn.executescript(sql)
            context, profile = ui._active_context_and_profile(conn)
            before = list(conn.iterdump())
            output.append({
                "group": group, "name": name, "sql": sql,
                "context": context, "profile": dict(profile) if profile else None,
                "health": ui.build_workspace_health_snapshot(conn),
                "next_actions": ui.build_next_actions_snapshot(conn),
                "badges": ui.build_review_badges_snapshot(conn),
                "coverage": ui.build_rates_coverage_snapshot(conn, {"limit": 1}),
                "overview": ui.build_overview_snapshot(conn),
                "workspace_overview": workspace_overview(conn),
            })
            assert list(conn.iterdump()) == before
            assert conn.execute("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").fetchone()[0] == 116
            conn.close()
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument(
        "--projections",
        help="comma-separated projections to refresh (e.g. health,next_actions); "
        "the others keep their accepted values, so slices of a main sync can "
        "regenerate only what they port",
    )
    args = parser.parse_args()
    generated = generate()
    if args.projections:
        keep = json.loads(DESTINATION.read_text())
        assert [(c["group"], c["name"]) for c in keep] == [(c["group"], c["name"]) for c in generated]
        for accepted, fresh in zip(keep, generated):
            for projection in args.projections.split(","):
                accepted[projection] = fresh[projection]
        generated = keep
    encoded = json.dumps(generated, indent=2, ensure_ascii=True) + "\n"
    if args.check:
        if DESTINATION.read_text() != encoded:
            raise SystemExit("workspace vectors differ; regenerate export_workspace_vectors.py")
    else:
        DESTINATION.write_text(encoded)
