#!/usr/bin/env python3
"""Freeze the desktop page contract from Python 3.11 on a disposable database."""
import argparse
import json
from pathlib import Path
import tempfile

from kassiber.core import ui_snapshot as ui
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.envelope import build_error_envelope

ROOT = Path(__file__).resolve().parents[3]
DATA = ROOT / "crates/transactions/tests/data"


def generate():
    output = []
    with tempfile.TemporaryDirectory(prefix="kassiber-desktop-vectors-") as root:
        conn = open_db(root)
        conn.executescript((DATA / "desktop.sql").read_text())

        def add(group, args, *, kind="list", setup=""):
            function = getattr(ui, f"build_transactions_{'snapshot' if kind == 'list' else kind + '_snapshot'}")
            try:
                expected = function(conn, args)
                if expected.get("txs") == []:
                    expected["year"] = "$utc_year"
                error = False
            except AppError as exc:
                expected = build_error_envelope(exc.code, str(exc), details=exc.details, hint=exc.hint, retryable=exc.retryable)
                error = True
            output.append(dict(group=group, kind="ui.transactions." + kind, args=args, setup=setup, expected=expected, error=error))
            return expected

        for args in ({}, None, [], {"query": ""}, {"query": 3}, {"query": " "},
                     {"query": "a", "z": 1, "extra": 2}, {"query": " a "},
                     {"query": "A"}, {"query": "external-E"}, {"query": "PAYMENT-HASH"},
                     {"query": "absent"}, {"query": "foreign"}, {"query": "d"}):
            add("resolve", args, kind="resolve")
        uppercase = "UPDATE transactions SET external_id='PAYMENT-HASH' WHERE id='b';"
        conn.executescript(uppercase)
        add("resolve", {"query": "payment-hash"}, kind="resolve", setup=uppercase)
        conn.execute("UPDATE transactions SET external_id='payment-hash' WHERE id='b'")
        aliases = "UPDATE transactions SET external_id='a' WHERE id='e';"
        conn.executescript(aliases)
        add("resolve", {"query": "a"}, kind="resolve", setup=aliases)
        conn.execute("UPDATE transactions SET external_id='external-e' WHERE id='e'")
        duplicate = "UPDATE transactions SET external_id='duplicate' WHERE id IN ('b','e','g');"
        conn.executescript(duplicate)
        add("resolve", {"query": "duplicate"}, kind="resolve", setup=duplicate)
        conn.execute("UPDATE transactions SET external_id=CASE id WHEN 'b' THEN 'payment-hash' WHEN 'e' THEN 'external-e' WHEN 'g' THEN 'external-g' ELSE external_id END")
        case_ids = "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at) VALUES('A','w','p','cold','case-id','BTC','inbound',2001,0,'2020','2020');"
        conn.executescript(case_ids)
        add("resolve", {"query": "A"}, kind="resolve", setup=case_ids)
        conn.execute("DELETE FROM transactions WHERE id='A'")
        for key, values in (
            ("wallet", ["Cold", "cold", "ln", "absent"]),
            ("direction", ["inbound", "outbound"]), ("asset", ["btc", "LBTC", "ß"]),
            ("txids", ["A,a,,external-f", ["B", "b", "external-e"]]),
            ("query", ["First", "last", "merchant", "memo", "word", "withdrawal", "payment-HASH", "Cold", "identifier-only-record", "unique-description-query", "a", "%"]),
            ("since", ["2025-02-01T00:00:00.000001Z", "2025-02-01T02:00:00.000001+02:00", "2025-02-01"]),
            ("start", ["2025-02-01T00:00:00.000001Z"]),
            ("end", ["2025-02-01T00:00:00.000001Z"]),
            ("until", ["2025-02-01T00:00:00.000001Z", "2025-02-01T02:00:00.000001+02:00", "2025-02-01"]),
            ("period", ["all", "max"]),
        ):
            for value in values:
                add("filters", {key: value})
        add("pages", {"limit": 1000})
        for key, values in (
            ("status", ["completed", "pending", "failed", "review", "needs-review", "error"]),
            ("flow", ["incoming", "outgoing", "transfer", "swap", "layer_transition"]),
            ("paymentMethod", ["On-chain", "Exchange", "Lightning", "Liquid"]),
            ("network", ["onchain", "Lightning", "liquid", "kraken", "btc", "regtest"]),
            ("withFees", [True, False, "yes", 1.0]),
            ("quick", ["external_flow", "review_queue", "no_explorer_id", "missing_price", "failed_import"]),
        ):
            for value in values:
                add("flows", {key: value})
        for flow in ("transfer", "swap", "layer-transition", "incoming"):
            add("flows", {"flow": flow, "candidate_txids": ["a", "c"]})
        for sort in ("occurred-at", "amount", "fee", "fiat-value"):
            for order in ("asc", "desc"):
                args = dict(limit=2, sort=sort, order=order)
                page = add("pages", args)
                first_cursor = page["nextCursor"]
                while page["nextCursor"]:
                    page = add("pages", {**args, "cursor": page["nextCursor"]})
                for change in ({"wallet": "ln"}, {"with_fees": True}, {"order": "desc" if order == "asc" else "asc"}, {"sort": "fee" if sort == "amount" else "amount"}):
                    add("pages", {**args, "cursor": first_cursor, **change})
        for args in ({}, {"limit": 1}, {"limit": 100}, {"direction": "outbound"}, {"wallet": "Cold"}, {"asset": "LBTC"}, {"since": "2025-02-01T00:00:00.000001Z"}):
            add("extremes", args, kind="extremes")
        for args in ({"query": "%"}, {"query": "deposit", "limit": 1000}, {"query": "First"}, {}):
            add("search", args, kind="search")
        for args in ({"limit": 2.9}, {"limit": True}, {"txids": [str(i) for i in range(129)]}, {"candidate_txids": [str(i) for i in range(513)]}):
            add("refusals", args)
        for args in ({"with_fees": False, "withFees": True}, {"payment_method": "Liquid", "paymentMethod": "Lightning"}, {"network": "Liquid"}):
            add("flows", args)
        for args in ({"sort": []}, {"sort": {}}, {"order": []}, {"order": {}}, {"sort": "bad"}, {"order": "bad"}, {"direction": "bad"}, {"query": " "}, {"limit": 0}, {"limit": None}, {"limit": "bad"}, {"cursor": "bad"}, {"withFees": None}, {"unknown": True}, {"txids": [1]}, {"status": "bad"}, {"flow": "bad"}, {"quick": "bad"}, {"payment_method": "bad"}, {"period": "bad"}, {"since": "bad"}, {"until": "bad"}):
            add("refusals", args)
        setup = """INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,created_at) VALUES('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','w','p','conversion','g','b','BTC','LBTC',100001,100001,'eligible','2025','2025'); UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=7,journal_input_version=2,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=2 WHERE id='p';"""
        conn.executescript(setup)
        for query in ("b", "B", "g", "payment-hash", "external-g"):
            add("resolve", {"query": query}, kind="resolve", setup=setup)
        for args in ({}, {"status": "review"}, {"status": "completed", "wallet": "ln"}, {"quick": "review_queue"}, {"quick": "failed_import"}, {"flow": "swap"}, {"flow": "incoming"}, {"since": "2025-02-01", "status": "completed"}):
            add("pairs", args, setup=setup)
        args = {"limit": 1}
        page = add("pairs", args, setup=setup)
        while page["nextCursor"]:
            page = add("pairs", {**args, "cursor": page["nextCursor"]}, setup=setup)
        for marker in ("last_processed_tx_count=0", "last_processed_input_version=1", "last_processed_at=NULL"):
            change = f"UPDATE profiles SET {marker} WHERE id='p';"
            conn.executescript(change)
            add("resolve", {"query": "b"}, kind="resolve", setup=setup + change)
            for flow in ("swap", "incoming", "transfer"):
                add("freshness", {"flow": flow}, setup=setup + change)
            conn.executescript("DELETE FROM journal_custody_economic_relations;" + setup)
        conn.execute("DELETE FROM journal_custody_economic_relations")
        move = """INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,occurred_at,created_at) VALUES('bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','w','p','g','b','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,100001,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,100001,'main','main','onchain','onchain','BTC','BTC','internal_reviewed','eligible','test','2025','2025'); UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=7,journal_input_version=2,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=2 WHERE id='p';"""
        conn.executescript(move)
        for query in ("b", "g", "payment-hash", "external-g"):
            add("resolve", {"query": query}, kind="resolve", setup=move)
        for args in ({}, {"flow": "transfer"}, {"flow": "incoming"}, {"wallet": "ln", "status": "completed"}, {"status": "review"}):
            add("moves", args, setup=move)
        conn.execute("DELETE FROM journal_custody_decisions")
        journal = """INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,description,created_at) VALUES('jin','w','p','b','ln','2025','transfer_in','BTC',100001,'transfer','2025'),('jout','w','p','g','cold','2025','transfer_out','BTC',-100001,'transfer','2025'); UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=7,journal_input_version=2,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=2 WHERE id='p';"""
        conn.executescript(journal)
        for query in ("b", "payment-hash"):
            add("resolve", {"query": query}, kind="resolve", setup=journal)
        add("moves", {}, setup=journal)
        conn.close()
    return json.dumps(output, ensure_ascii=True, separators=(",", ":")) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = DATA / "desktop-vectors.json"
    source = generate()
    if args.check:
        return int(path.read_text() != source)
    path.write_text(source)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
