"""Actual multi-asset engine carrying dependencies, not hand-built journal rows."""
from contextlib import closing
from decimal import Decimal
import json

import pytest

from kassiber.db import open_db
from kassiber.cli.handlers import create_transaction_pair, process_journals
from kassiber.core import custody_filed_reports as reports, filed_report_chain as impacts
from tests.custody_tax_helpers import persist_authoritative_chain_observation


@pytest.mark.parametrize("policy", ["carrying-value", "taxable"])
def test_cross_rail_dependency_closure_follows_actual_reviewed_basis(tmp_path, policy):
    with closing(open_db(tmp_path / "book")) as conn:
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,created_at) VALUES('p','w','P','USD','generic','FIFO','2020')")
        for wallet, chain in [("btc", "bitcoin"), ("liquid", "liquid")]:
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,'descriptor',?,'2020')",
                         (wallet, wallet, json.dumps({"chain": chain, "network": "main"})))
        rows = [
            ("buy", "btc", "2024-01-01", "inbound", "BTC", 10_010_000_000, 0, 80_000, "buy"),
            ("swap-out", "btc", "2024-02-01", "outbound", "BTC", 10_000_000_000, 10_000_000, 82_000, "withdrawal"),
            ("swap-in", "liquid", "2024-02-01", "inbound", "LBTC", 10_000_000_000, 0, 82_000, "deposit"),
            ("sell", "liquid", "2025-01-01", "outbound", "LBTC", 10_000_000_000, 0, 83_000, "sell"),
            # Even inside the report year, this later source-asset purchase
            # cannot affect the earlier reviewed carrying-value result.
            ("later-btc", "btc", "2025-06-01", "inbound", "BTC", 1_000_000_000, 0, 90_000, "buy"),
        ]
        for index, (ident, wallet, date, direction, asset, amount, fee, rate, kind) in enumerate(rows, start=1):
            txid = f"{index:064x}"
            chain = "bitcoin" if wallet == "btc" else "liquid"
            raw = {"txid": txid, "chain": chain, "network": "main",
                   "status": {"confirmed": True, "block_hash": "11" * 32, "block_height": 100},
                   "vin": [{"txid": f"{index+5:064x}", "vout": 0, "prevout": {"scriptpubkey": "0014" + "ff" * 20, "value": 10_000_000}}],
                   "vout": [{"n": 0, "scriptpubkey": "0014" + "ee" * 20, "value": 10_000_000}]}
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,fingerprint,occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate,fiat_rate_exact,kind,raw_json,created_at) VALUES(?,'w','p',?,?,'txid',?,?,?,?,?,?,'USD',?,?,?,?,'2020')",
                         (ident, wallet, txid, ident, date + "T00:00:00Z", direction, asset, amount, fee, rate, str(rate), kind, json.dumps(raw)))
            persist_authoritative_chain_observation(conn, ident, observer_kind="bitcoinrpc" if chain == "bitcoin" else "lwk")
        conn.commit()
        create_transaction_pair(conn, "W", "P", "swap-out", "swap-in", kind="peg-in", policy=policy)
        process_journals(conn, "W", "P")
        assert not conn.execute("SELECT reason FROM journal_quarantines").fetchall()
        if policy == "carrying-value":
            gain = conn.execute("SELECT gain_loss_exact FROM journal_entries WHERE transaction_id='sell'").fetchone()[0]
            assert Decimal(gain) == Decimal("292")
        artifact = tmp_path / "report.csv"
        artifact.write_text("completed export fixture")
        reports.register_saved_report_export(conn, workspace_id="w", profile_id="p", report_kind="full-report.csv",
            artifact_paths=[artifact], period_start_year=2025, period_end_year=2025, report_scope={"wallet_ids": ["liquid"]})
        dependencies = {row[0] for row in conn.execute("SELECT transaction_id FROM filed_report_chain_dependencies")}
        assert dependencies == ({"buy", "swap-out", "swap-in", "sell"} if policy == "carrying-value" else {"swap-in", "sell"})
        # Replace the acquisition's confirmed block with fresh native authority.
        raw = json.loads(conn.execute("SELECT raw_json FROM transactions WHERE id='buy'").fetchone()[0])
        raw["status"]["block_hash"] = "22" * 32
        conn.execute("UPDATE transactions SET raw_json=? WHERE id='buy'", (json.dumps(raw),))
        persist_authoritative_chain_observation(conn, "buy")
        assert impacts.record_wallet_changes(conn, "p", "btc") == (1 if policy == "carrying-value" else 0)
