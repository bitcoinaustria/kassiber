#!/usr/bin/env python3
"""Freeze synthetic network evidence, inventory, binding, and partition previews."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import sys
from unittest import mock
import uuid

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import book_network as network, book_network_migration as migration
from kassiber.errors import AppError
from tests.test_transaction_graph import TransactionGraphTest

DESTINATION = Path(__file__).resolve().parents[1] / "fixtures/python_networks.json"
SCOPES = Path(__file__).resolve().parents[1] / "src/network_scopes.rs"


def scope_source():
    scopes = {table: sql for table, sql in migration.AUTHORED_SCOPES.items()
              if table not in migration.HISTORICAL_ONLY}
    scopes.update(migration.EXTRA_SCOPES)
    entries = "".join(f"    ({json.dumps(table)}, {json.dumps(sql)}),\n"
                      for table, sql in scopes.items())
    return ("// Python reference: book_network_migration.AUTHORED_SCOPES/EXTRA_SCOPES.\n"
            "pub const SCOPES: &[(&str, &str)] = &[\n" + entries + "];\n")


def generate():
    records = []
    for scenario in ("unknown", "known", "mixed", "contradictory", "reference", "partition", "legacy", "bound", "binance"):
        book = TransactionGraphTest()
        book.setUp()
        try:
            conn = book.conn
            if scenario in {"known", "bound"}:
                conn.execute("UPDATE wallets SET config_json=?", (json.dumps({"chain": "bitcoin", "network": "main"}),))
            if scenario in {"mixed", "contradictory", "partition", "legacy"}:
                book._tx("main", "wallet-a", "inbound", 1000000, "a" * 64, {"network": "main"})
                book._tx("other", "wallet-b", "inbound", 1000000, "b" * 64, {"network": "regtest"})
            if scenario == "binance":
                # Binance's "BTC" rail is mainnet evidence in a Binance
                # wallet only; elsewhere the code contradicts the row.
                conn.execute("UPDATE wallets SET kind='binance' WHERE id='wallet-a'")
                book._tx("rail", "wallet-a", "outbound", 1000000, "a" * 64, {"network": "BTC"})
                book._tx("other", "wallet-b", "inbound", 1000000, "b" * 64, {"network": "BTC"})
            if scenario == "contradictory":
                conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-b'", (json.dumps({"network": "main"}),))
            if scenario == "reference":
                conn.execute("INSERT INTO chain_analysis_observations VALUES(?,?,?,?,?,?,?,?)", ("profile-1", "bitcoin", "regtest", "a" * 64, "{}", "{}", "local", "2026-01-01"))
            if scenario == "legacy":
                conn.execute("CREATE TABLE gl_books(profile_id TEXT PRIMARY KEY,created_at TEXT)")
                conn.execute("INSERT INTO gl_books VALUES('profile-1','2026-01-01')")
            if scenario == "bound":
                args = {"environment": "main"}
                plan = network.plan_book_network(conn,"profile-1",args)
                with mock.patch("uuid.uuid4", return_value=uuid.UUID(int=7)), mock.patch.object(network,"now_iso",return_value="2026-01-01T00:00:00Z"):
                    network.apply_book_network(conn,"profile-1",{**args,"plan_id":plan["plan_id"]})
            conn.commit()
            allowed = set(migration.AUTHORED_SCOPES) | set(migration.EXTRA_SCOPES) | {"chain_analysis_observations", "chain_observer_instances", "wallet_utxos", "gl_books"}
            seed = [line for line in conn.iterdump() if line.startswith("CREATE TABLE gl_") or line.startswith("INSERT INTO") and line.split('"', 2)[1] in allowed]
            operations = [("inventory",{})]
            for environment in ("main", "test", "signet", "regtest"):
                for declared in ([], ["wallet-c","wallet-b","wallet-a","wallet-a"]):
                    args = {"environment":environment,"declared_wallet_ids":declared}
                    if environment == "regtest": args["chain_instance_id"] = "URN-INVALID" if not declared else "{00000000000000000000000000000001}"
                    operations.append(("plan",args))
            operations += [("plan",{"environment":"main","declared_wallet_ids":["absent"]}), ("plan",{"environment":"main","declared_wallet_ids":None}), ("plan",{"environment":"main","chain_instance_id":"unexpected"}), ("plan",{"environment":"invalid"})]
            for selected in (["wallet-a"], ["wallet-a","wallet-b"], [], ["absent"]):
                operations.append(("split-plan",{"environment":"main","wallet_ids":selected,"declared_wallet_ids":["wallet-a"] if selected else []}))
            cases = []
            for operation,args in operations:
                try:
                    result = {"ok": ({"inventory": network.inventory_book_network, "plan": network.plan_book_network,"split-plan":migration.plan_network_partition}[operation])(conn,"profile-1",*([args] if operation!="inventory" else []))}
                except AppError as exc:
                    result = {"error":{"code":exc.code,"message":str(exc),"details":exc.details,"hint":exc.hint,"retryable":exc.retryable}}
                cases.append({"operation":operation,"args":args,**result})
            records.append({"scenario":scenario,"seed":seed,"cases":cases})
        finally:
            book.tearDown()
    evidence = []
    for chain in (None,"bitcoin","liquid","lightning","ln","core_ln","unsupported"):
        for value in (None,"mainnet","testnet","signet","elementsregtest","unsupported",{},0,False):
            row={"config_json":{"chain":chain,"network":value}}
            evidence.append({"row":row,"expected":network._evidence(row)})
    for row in [
        {"wallet_kind":"coinbase","raw_json":{"network":{"hash":"receipt"}}},
        *({"raw_json":{"source":"bullbitcoin_wallet_csv","network":rail}} for rail in ("bitcoin","liquid","lightning")),
        {"raw_json":{"network":"main","chain_instance_id":"one"},"wallet_config_json":{"network":"regtest","chain_instance_id":"two"}},
        {"chain":"bitcoin","network":"regtest","raw_json":{"network":"main"}},
        *({"wallet_kind":kind,"asset":"BTC","raw_json":{"network":rail}} for kind in ("binance","custom",None) for rail in ("BTC","btc"," BTC","LIGHTNING","BSC","BEP20",{"name":"BTC"},None)),
        {"wallet_kind":"binance","asset":"BTC","raw_json":{"network":"BTC"},"wallet_config_json":{"network":"regtest"}},
        {"wallet_kind":"binance","asset":"BTC","raw_json":{"network":"BTC"},"wallet_config_json":{"network":"main"}},
        *({"wallet_kind":"binance","asset":asset,"raw_json":{"network":"BTC"}} for asset in ("LBTC","btc",None)),
        {"wallet_kind":"binance","asset":"BTC","network":"regtest","raw_json":{"network":"BTC"}},
    ]:
        evidence.append({"row":row,"expected":network._evidence(row)})
    return json.dumps({"scenarios":records,"evidence":evidence},ensure_ascii=True,indent=2)+"\n"


if __name__ == "__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("--check",action="store_true")
    args=parser.parse_args()
    generated=generate()
    if args.check:
        if generated != DESTINATION.read_text(): raise SystemExit("network vectors have drifted")
        if scope_source() != SCOPES.read_text(): raise SystemExit("network partition scopes have drifted")
    else:
        DESTINATION.write_text(generated)
        SCOPES.write_text(scope_source())
