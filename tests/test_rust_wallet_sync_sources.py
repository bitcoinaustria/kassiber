"""Wallet sync for file sources and Bull Bitcoin export routes against unchanged Python.

Books are disposable; every child gets its own XDG roots. These routes read
local files only, so the egress switches stay on throughout.
"""
import json
import os
from pathlib import Path

import pytest

import subprocess

from tests.rust_port.hybrid_cli import SYNC_HANDOFF
from tests.test_cli_smoke import _BULLBITCOIN_ORDERS_CSV, _TWENTYONEBITCOIN_TRANSACTIONS_CSV
from tests.test_rust_connections import (
    PASSPHRASE, RUST_BIN, btcpay_book, bull_export, masked, masked_dump, run, served, started_stamp,
)

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")

CSV = ("txid,occurred_at,direction,asset,amount,fee,description\n"
       "{tx1},2026-01-02T10:00:00Z,inbound,BTC,0.0015,0.000002,Salary\n"
       "csv-2,2026-03-02T00:00:00Z,outbound,BTC,0.0005,0.00001,Rent\n")
JSON = [{"txid": "json-1", "occurred_at": "2026-02-01T00:00:00Z", "direction": "inbound", "asset": "BTC",
         "amount": 0.25, "fee": 0, "fiat_rate": 40000, "description": "Funding"}]


def sources_book(tmp_path, *, encrypted=False):
    exports = tmp_path / "exports"
    exports.mkdir()
    bull = bull_export(exports / "bull.csv")
    tx1 = "1" * 64
    (exports / "wallet.csv").write_text(CSV.format(tx1=tx1))
    (exports / "wallet.json").write_text(json.dumps(JSON))
    (exports / "orders.csv").write_text(_BULLBITCOIN_ORDERS_CSV)
    (exports / "21.csv").write_text(_TWENTYONEBITCOIN_TRANSACTIONS_CSV)
    wallets = [
        ("csvw", "CSV", "custom", {"source_file": str(exports / "wallet.csv"), "source_format": "csv",
                                    "bullbitcoin_wallet_exports": [{"source_file": bull, "network": "bitcoin"},
                                                                   {"source_file": str(exports / "x" / ".." / "bull.csv"), "network": "BTC"}]}),
        ("jsonw", "JSON", "custom", {"source_file": str(exports / "wallet.json"), "source_format": "json"}),
        ("bullb", "Bull - Bitcoin", "bullbitcoin", {"source_file": bull, "source_format": "bullbitcoin_wallet_csv",
                                                     "bullbitcoin_wallet_network": "bitcoin", "chain": "bitcoin"}),
        ("bulll", "Bull - Liquid", "bullbitcoin", {"source_file": bull, "source_format": "bullbitcoin_wallet_csv",
                                                    "bullbitcoin_wallet_network": "liquid", "chain": "liquid"}),
        ("orders", "Orders", "custom", {"source_file": str(exports / "orders.csv"), "source_format": "bullbitcoin_csv"}),
        ("ordersf", "Orders Full", "custom", {"source_file": str(exports / "orders.csv"),
                                              "source_format": "bullbitcoin_csv", "import_mode": "full"}),
        ("t21", "21", "21bitcoin", {"source_file": str(exports / "21.csv"), "source_format": "21bitcoin_csv"}),
        ("missing", "Missing", "custom", {"source_file": str(exports / "gone.csv"), "source_format": "csv"}),
        ("unknown", "Unknown", "custom", {"source_file": str(exports / "wallet.csv"), "source_format": "doge_csv"}),
        ("half", "Half", "custom", {"source_file": str(exports / "wallet.csv")}),
        ("routes", "Routes only", "custom", {"bullbitcoin_wallet_exports": [{"source_file": bull, "network": "bitcoin"}]}),
        ("badroute", "Bad route", "custom", {"source_file": str(exports / "wallet.json"), "source_format": "json",
                                             "bullbitcoin_wallet_exports": "nope"}),
    ]
    sql = "".join(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES"
        f"('{wallet_id}','w','p','{label}','{kind}','{json.dumps(config)}','2026-01-01T00:00:00Z');"
        for wallet_id, label, kind, config in wallets)
    sql += "DELETE FROM wallets WHERE id='a';"
    root, env = btcpay_book(tmp_path, encrypted=encrypted, sql=sql)
    return root, env, [label for _, label, _, _ in wallets]


def native(root, argv, env):
    """The Rust binary itself: the hybrid keeps passphrase-fd syncs on Python."""
    read_fd, write_fd = os.pipe()
    os.write(write_fd, PASSPHRASE.encode())
    os.close(write_fd)
    try:
        return subprocess.run([str(Path(RUST_BIN).resolve()), "--data-root", str(root), "--db-passphrase-fd", str(read_fd),
                               *argv], capture_output=True, env=env, timeout=600, pass_fds=(read_fd,))
    finally:
        os.close(read_fd)


def compare_cli(root, env, argv, *, encrypted=False):
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    started = started_stamp()
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(initial)
        if encrypted and not reference:
            result = native(root, ["--machine", *argv], env)
        else:
            result = run(root, ["--machine", *argv], env, reference=reference, passphrase=encrypted)
        outcomes.append((result.returncode, masked(result.stdout.replace(str(root).encode(), b"ROOT"), started),
                         result.stderr, masked_dump(root, started, encrypted)))
    assert outcomes[1][:3] == outcomes[0][:3], argv
    assert outcomes[1][3] == outcomes[0][3], argv
    return json.loads(outcomes[1][1]), outcomes[1][3]


def test_file_sources_sync_all_twice_matches_python(tmp_path):
    root, env, _ = sources_book(tmp_path)
    first, tables = compare_cli(root, env, ["wallets", "sync", "--all"])
    rows = {row["wallet"]: row for row in first["data"]}
    assert rows["CSV"]["status"] == "synced" and rows["CSV"]["bullbitcoin_wallet_exports"]["routes"] == 1
    assert rows["CSV"]["bullbitcoin_wallet_exports"]["updated"] == 1, rows["CSV"]
    assert rows["Bull - Bitcoin"]["bullbitcoin_wallet_network"] == "bitcoin"
    assert rows["Missing"]["status"] == "error" and rows["Unknown"]["status"] == "error"
    assert rows["Half"]["status"] == "skipped" and rows["Routes only"]["status"] == "skipped"
    assert rows["Bad route"]["code"] == "validation"
    assert sum(b"last_synced_at" in row for row in tables["wallets"]) == 6
    second, _ = compare_cli(root, env, ["wallets", "sync", "--all"])
    assert {row["wallet"]: row.get("imported") for row in second["data"]}["CSV"] == 0
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_each_file_source_wallet_matches_python(tmp_path, encrypted):
    root, env, labels = sources_book(tmp_path, encrypted=encrypted)
    for label in labels:
        compare_cli(root, env, ["wallets", "sync", "--wallet", label], encrypted=encrypted)
    if not encrypted:
        log = served(env)
        assert len(log) == len(labels) and {entry["served_by"] for entry in log} == {"rust"}, log


# A silent-payment wallet whose backend is Core RPC: still Python's (rust-native-wallet-gaps.md, S6).
def test_unported_wallet_routes_hand_off_natively_before_any_write(tmp_path):
    """A wallet route the native service does not take (a descriptor wallet
    on a custom backend, which only scans Silent Payments) gets the
    pre-consent handoff."""
    import sqlite3
    from tests.test_rust_wallet_publication import _wallet

    root, env, _ = sources_book(tmp_path)
    config, _ = _wallet("wpkh", 3)
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    conn.execute("UPDATE wallets SET kind='descriptor', config_json=? WHERE id='jsonw'",
                 (json.dumps({**config, "backend": "local"}),))
    conn.execute("INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) "
                 "VALUES('local','custom','local://scanner','',2,'{}','fixed','fixed')")
    conn.commit()
    conn.close()
    before = (root / "kassiber.sqlite3").read_bytes()
    argv = [str(Path(RUST_BIN).resolve()), "--machine", "--data-root", str(root), "wallets", "sync", "--wallet", "JSON"]
    handoff = subprocess.run(argv, capture_output=True, env={**env, "KASSIBER_RUST_TEST_SYNC_HANDOFF": "1"}, timeout=600)
    assert (handoff.returncode, handoff.stdout, handoff.stderr) == (SYNC_HANDOFF, b"", b"")
    refused = subprocess.run(argv, capture_output=True, env=env, timeout=600)
    assert json.loads(refused.stdout)["error"]["code"] == "native_fallback_required", refused
    assert json.loads(refused.stdout)["error"]["details"] == {"reason": "backend_kind"}
    assert (root / "kassiber.sqlite3").read_bytes() == before
