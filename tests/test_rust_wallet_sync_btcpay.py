"""BTCPay wallet sources, provenance enrichment and `wallets sync-btcpay` against unchanged Python.

Books are disposable; every child gets its own XDG roots. Greenfield is a
loopback fake; the egress switches prove no request leaves before consent.
"""
import json
from pathlib import Path
import sqlite3

import pytest

from tests.rust_sync_parity import fixed_elapsed_ms_bytes
from tests.test_rust_connections import (
    PAY_TXID, PAYOUT_TXID, RUST_BIN, SECRET, btcpay_server, masked, masked_dump, provenance_book,
    provenance_routes, run, served, started_stamp,
)

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")

HISTORY = [
    {"transactionHash": PAY_TXID, "amount": "0.001", "timestamp": 1700000100, "confirmations": 3,
     "comment": "Invoice inv-paid", "labels": {"invoice": {"type": "invoice", "text": "inv-paid"}}},
    {"transactionHash": PAYOUT_TXID, "amount": "-0.0006", "timestamp": 1700000500, "confirmations": "2",
     "comment": "", "labels": ["payout", "Refund Batch"]},
    {"transactionHash": "ee" * 32, "amount": "0.5", "timestamp": 1700000900, "confirmations": 0},
    {"transactionHash": "dd" * 32, "amount": 0.25, "timestamp": 1700000950, "status": "Confirmed"},
]


SCOPED = {
    "scoped": "btcpay.store.canmodifystoresettings:scoped",
    "otherstore": "btcpay.store.canmodifystoresettings:scoped",
    "emptyscope": "btcpay.store.canmodifystoresettings:",
    "viewscoped": "btcpay.store.canviewstoresettings:viewscoped",
    # BTCPay 2.4 names the read-only `canviewwallet` instead (issue #692).
    "walletscoped": "btcpay.store.canviewwallet:walletscoped",
    "walletother": "btcpay.store.canviewwallet:walletscoped",
    "managewallets": "btcpay.store.canmanagewallets:managewallets",
}


def paged(items):
    def route(query):
        skip = int(query.get("skip", ["0"])[0])
        limit = int(query.get("limit", ["100"])[0])
        return 200, items[skip:skip + limit]
    return route


def routes():
    result = provenance_routes()
    tx = "/api/v1/stores/{}/payment-methods/BTC-CHAIN/wallet/transactions"
    result[tx.format("shop")] = paged(HISTORY)
    result[tx.format("ro")] = (403, {"code": "missing-permission",
                                     "missingPermission": "btcpay.store.canmodifystoresettings"})
    # Only the route's own store scope marks a read-only key (Python r7-10).
    for store, missing in SCOPED.items():
        result[tx.format(store)] = (403, {"code": "missing-permission", "missingPermission": missing})
    result[tx.format("bare")] = (403, b"forbidden")
    result[tx.format("broken")] = (500, {"message": "boom"})
    for store in ("ro", "bare", "broken", *SCOPED):
        result[f"/api/v1/stores/{store}/invoices"] = (200, [])
        result[f"/api/v1/stores/{store}/payouts"] = (200, [])
    return result


def btcpay_wallets(tmp_path, url, *, encrypted=False):
    exports = tmp_path / "exports"
    exports.mkdir(exist_ok=True)
    csv = exports / "mapped.csv"
    csv.write_text("txid,occurred_at,direction,asset,amount,fee,description\n"
                   f"{PAY_TXID},2023-11-14T22:15:00Z,inbound,BTC,0.001,0,Mapped\n")
    route = lambda store: [{"backend": "merchant", "store_id": store, "payment_method_id": "btc-chain"}]
    wallets = [
        ("shop", "Shop", {"sync_source": "btcpay", "backend": "MERCHANT", "store_id": "shop"}),
        ("ledger", "Ledger", {"sync_source": "btcpay", "backend": "merchant", "store_id": "shop",
                              "source_mode": "payments", "payment_method_ids": ["BTC-LN", "btc-chain"]}),
        ("mapped", "Mapped", {"source_file": str(csv), "source_format": "csv", "btcpay_provenance": route("shop")}),
        ("ro", "Read only", {"source_file": str(csv), "source_format": "csv", "btcpay_provenance": route("ro")}),
        ("bare", "Bare 403", {"source_file": str(csv), "source_format": "csv", "btcpay_provenance": route("bare")}),
        *[(store, store.title(), {"source_file": str(csv), "source_format": "csv", "btcpay_provenance": route(store)})
          for store in SCOPED],
        ("kind", "Chain kind", {"sync_source": "btcpay", "backend": "chain", "store_id": "shop"}),
        ("broken", "Broken", {"sync_source": "btcpay", "backend": "merchant", "store_id": "broken"}),
    ]
    root, env = provenance_book(tmp_path, url, encrypted)
    conn = sqlite3.connect(root / "kassiber.sqlite3") if not encrypted else None
    if conn is None:
        from tests.test_rust_connections import connect
        conn = connect(root, True)
    for wallet_id, label, config in wallets:
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES(?,'w','p',?,'custom',?,'2026-01-01T00:00:00Z')", (wallet_id, label, json.dumps(config)))
    conn.commit()
    conn.close()
    return root, env, [label for _, label, _ in wallets]


def compare(root, env, hits, argv, url, *, encrypted=False):
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    started = started_stamp()
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(initial)
        hits.clear()
        result = run(root, ["--machine", *argv], env, reference=reference, passphrase=encrypted)
        assert SECRET.encode() not in result.stdout
        outcomes.append((result.returncode, masked(result.stdout.replace(url.encode(), b"URL"), started),
                         result.stderr, [path for path, _ in hits], masked_dump(root, started, encrypted)))
    assert outcomes[1][:4] == outcomes[0][:4], argv
    assert outcomes[1][4] == outcomes[0][4], argv
    return json.loads(outcomes[1][1]), outcomes[1][3]


def test_btcpay_sources_and_enrichment_sync_like_python(tmp_path):
    with btcpay_server(routes()) as (url, hits):
        root, env, labels = btcpay_wallets(tmp_path, url)
        for label in labels:
            compare(root, env, hits, ["wallets", "sync", "--wallet", label], url)
        first, requests = compare(root, env, hits, ["wallets", "sync", "--all"], url)
        rows = {row["wallet"]: row for row in first["data"]}
        assert rows["Shop"]["payout_fees"] == {"separated": 0, "backfilled": 0} or rows["Shop"]["status"] == "synced"
        assert rows["Ledger"]["source_mode"] == "payments", rows["Ledger"]
        assert rows["Mapped"]["btcpay_provenance"]["route_results"][0]["labels"] == "synced"
        for label in ("Read only", "Bare 403"):
            assert rows[label]["btcpay_provenance"]["labels_unavailable_routes"] == 1, rows[label]
        for label in ("Scoped", "Walletscoped"):
            assert rows[label]["btcpay_provenance"]["route_results"][0]["labels"] == "unavailable_read_only_key"
        for label in ("Otherstore", "Emptyscope", "Viewscoped", "Walletother", "Managewallets"):
            assert rows[label]["code"] == "auth_error", rows[label]
        assert rows["Chain kind"]["status"] == "error" and rows["Broken"]["status"] == "error"
        assert any("wallet/transactions" in path for path in requests)
        compare(root, env, hits, ["wallets", "sync", "--all"], url)
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


def test_btcpay_wallet_separates_payout_fees_like_python(tmp_path):
    with btcpay_server(routes()) as (url, hits):
        root, env, _ = btcpay_wallets(tmp_path, url)
        result, _ = compare(root, env, hits, ["wallets", "sync", "--wallet", "Shop"], url)
        assert result["data"][0]["payout_fees"]["separated"] == 1, result
        assert result["data"][0]["fetched"] == 3
        again, _ = compare(root, env, hits, ["wallets", "sync", "--wallet", "Shop"], url)
        assert again["data"][0]["imported"] == 0


@pytest.mark.parametrize("options", [
    ["--wallet", "Cold", "--backend", "merchant", "--store-id", "shop"],
    ["--wallet", "Cold", "--backend", "merchant", "--store-id", "shop", "--page-size", "1"],
    ["--wallet", "Cold", "--backend", "MERCHANT", "--store-id", " shop ", "--payment-method-id", "btc"],
    ["--wallet", "Ledger", "--backend", "merchant", "--store-id", "shop"],
    ["--wallet", "Cold", "--backend", "merchant", "--store-id", "shop", "--page-size", "0"],
    ["--wallet", "Cold", "--backend", "merchant", "--store-id", "broken"],
    ["--wallet", "Cold", "--backend", "merchant", "--store-id", "shop", "--payment-method-id", "BTC-LN"],
    ["--wallet", "Cold", "--backend", "merchant", "--store-id", " "],
    ["--wallet", "Cold", "--backend", "chain", "--store-id", "shop"],
    ["--wallet", "Cold", "--backend", "missing", "--store-id", "shop"],
    ["--wallet", "missing", "--backend", "merchant", "--store-id", "shop"],
])
def test_sync_btcpay_cli_matches_python(tmp_path, options):
    with btcpay_server(routes()) as (url, hits):
        root, env, _ = btcpay_wallets(tmp_path, url)
        compare(root, env, hits, ["wallets", "sync-btcpay", *options], url)
    assert served(env) == [{"command": "wallets.sync-btcpay", "served_by": "rust"}]


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_btcpay_sync_sqlcipher_and_plaintext_match_python(tmp_path, encrypted):
    with btcpay_server(routes()) as (url, hits):
        root, env, _ = btcpay_wallets(tmp_path, url, encrypted=encrypted)
        compare(root, env, hits, ["wallets", "sync-btcpay", "--wallet", "Cold", "--backend", "merchant",
                                  "--store-id", "shop"], url, encrypted=encrypted)


@pytest.mark.parametrize("switch", ["offline", "no-egress"])
@pytest.mark.parametrize("argv", [["wallets", "sync", "--wallet", "Shop"], ["wallets", "sync", "--wallet", "Mapped"],
                                  ["wallets", "sync-btcpay", "--wallet", "Cold", "--backend", "merchant", "--store-id", "shop"]])
def test_btcpay_sync_refuses_before_any_request(tmp_path, switch, argv):
    with btcpay_server(routes()) as (url, hits):
        root, env, _ = btcpay_wallets(tmp_path, url)
        if switch == "offline":
            Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text('{"enabled":true}')
        else:
            env["KASSIBER_NO_EGRESS"] = "1"
        result, requests = compare(root, env, hits, argv, url)
        assert requests == []
        assert result["error"]["code"] == "network_egress_disabled", result


@pytest.mark.parametrize("argv", [["--wallet", "Desc"], ["--all"]], ids=["wallet", "all"])
def test_btcpay_provenance_enriches_a_descriptor_wallet_like_python(tmp_path, argv):
    """A descriptor wallet's BTCPay provenance routes are read inside its
    publication savepoint after the chain sync: history comments and labels
    become notes and tags on the chain-observed transactions, and the store's
    provenance is refreshed. Generated ids and clocks are masked."""
    import copy

    from tests.rust_sync_chain import serve
    from tests.test_rust_wallet_publication import _fixture

    config, chain, _ = _fixture("receive-change")
    spend = list(chain.txs)[-1]
    parent = chain.txs[spend].vin[0].txid.hex()
    history = [{"transactionHash": parent, "amount": "0.001", "timestamp": 1700000100, "confirmations": 3,
                "comment": "Invoice paid", "labels": {"invoice": {"type": "invoice", "text": "inv-paid"}}},
               {"transactionHash": spend, "amount": "-0.00052", "timestamp": 1700000500, "confirmations": 2,
                "comment": "", "labels": ["payout", "Refund Batch"]}, *HISTORY]
    server = routes()
    server["/api/v1/stores/shop/payment-methods/BTC-CHAIN/wallet/transactions"] = paged(history)
    config.update({"backend": "chain",
                   "btcpay_provenance": [{"backend": "merchant", "store_id": "shop", "payment_method_id": "btc-chain"}]})
    with btcpay_server(server) as (url, hits), serve(chain, "esplora") as chain_url:
        root, env = provenance_book(tmp_path, url)
        conn = sqlite3.connect(root / "kassiber.sqlite3")
        conn.execute("UPDATE backends SET url=? WHERE name='chain'", (chain_url,))
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES('d','w','p','Desc','descriptor',?,'2026-01-01T00:00:00Z')", (json.dumps(config),))
        conn.commit()
        conn.close()
        database = root / "kassiber.sqlite3"
        for _round in range(2):
            initial = database.read_bytes()
            state = copy.deepcopy({k: v for k, v in chain.__dict__.items() if k not in {"lock", "hits"}})
            started = started_stamp()
            outcomes = []
            for reference in [True, False]:
                database.write_bytes(initial)
                with chain.lock:
                    chain.__dict__.update(copy.deepcopy(state))
                hits.clear()
                result = run(root, ["--machine", "wallets", "sync", *argv], env, reference=reference)
                assert SECRET.encode() not in result.stdout
                stdout = masked(result.stdout.replace(url.encode(), b"URL").replace(chain_url.encode(), b"CHAIN"), started)
                outcomes.append((result.returncode, fixed_elapsed_ms_bytes(stdout),
                                 result.stderr, sorted(path for path, _ in hits), masked_dump(root, started)))
            # Envelopes compare as JSON: key order is not semantic (rust-shell.md).
            assert (outcomes[1][0], outcomes[1][2], outcomes[1][3]) == (outcomes[0][0], outcomes[0][2], outcomes[0][3])
            from tests.rust_port.json_diff import json_diff
            assert json.loads(outcomes[1][1]) == json.loads(outcomes[0][1]), json_diff(
                json.loads(outcomes[0][1]), json.loads(outcomes[1][1]))
            if _round == 0:
                rows = {row["wallet"]: row for row in json.loads(outcomes[1][1])["data"]}
            assert outcomes[1][4] == outcomes[0][4]
    enriched = rows["Desc"]["btcpay_provenance"]
    assert rows["Desc"]["status"] == "synced", rows["Desc"]
    assert (enriched["routes"], enriched["fetched"], enriched["btcpay_tags_added"]) == (1, len(history) - 1, 3), enriched  # one row is unconfirmed
    assert {entry["served_by"] for entry in served(env)} == {"rust"}
