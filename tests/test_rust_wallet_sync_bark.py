"""Bark (BTC-ARK) wallets through BTCPay against unchanged Python: the wallet
import, the switch from a payment ledger, Lightning rows moving out of a
BTC-LN ledger, and every refusal. Books are disposable; Greenfield is a
loopback fake; every table is compared. CLI cases run on plaintext books (the
hybrid hands an encrypted `wallets sync` to Python); SQLCipher books run the
same syncs as the native daemon's freshness jobs.
"""
import json
import sqlite3

import pytest

from tests.test_btcpay_bark import H_BTC_LN, H_PAYOUT, INVOICES, ROWS, _invoice, _row
from tests.test_rust_connections import OWNED_A, RUST_BIN, btcpay_book, btcpay_server, compare_cli_masked, connect, served
from tests.test_rust_wallet_sync_btcpay import compare

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")

STORE = "/api/v1/stores/shop"
BARK = {"sync_source": "btcpay", "backend": "merchant", "store_id": "shop", "payment_method_id": "BTC-ARK", "chain": "bitcoin"}
ARK_LEDGER = {**BARK, "source_mode": "payments", "payment_method_ids": ["BTC-ARK"]}
LN_LEDGER = {**BARK, "source_mode": "payments", "payment_method_id": "BTC-LN", "payment_method_ids": ["BTC-LN"]}


def _paged(rows):
    def route(query):
        statuses = query.get("statusFilter") or []
        selected = [row for row in rows if not statuses or not isinstance(row, dict) or row.get("status") in statuses]
        skip = int(query.get("skip", ["0"])[0])
        limit = int(query.get("limit", ["100"])[0])
        return 200, selected[skip : skip + limit]

    return route


def _invoices(invoices):
    return lambda query: (200, invoices if query.get("skip") == ["0"] else [])


def routes(*, rows=ROWS, invoices=INVOICES, served="yes", payouts=()):
    result = {
        STORE + "/invoices": _invoices(invoices),
        STORE + "/payouts": (200, list(payouts)),
        STORE + "/pull-payments": (200, []),
        STORE + "/payment-requests": (200, []),
    }
    if served == "yes":
        result[STORE + "/payment-methods/BTC-ARK/wallet"] = (200, {"balance": "0", "label": ""})
        result[STORE + "/payment-methods/BTC-ARK/wallet/transactions"] = _paged(rows)
    elif served == "permission":
        result[STORE + "/payment-methods/BTC-ARK/wallet"] = (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewwallet"})
    elif served == "unset":
        result[STORE + "/payment-methods/BTC-ARK/wallet"] = (404, {"code": "paymentmethod-not-found"})
    elif served == "busy":
        result[STORE + "/payment-methods/BTC-ARK/wallet"] = (503, {"code": "not-available"})
    return result


def book(tmp_path, url, wallets, *, encrypted=False, sql=""):
    root, env = btcpay_book(tmp_path, encrypted=encrypted, url=url, sql=sql)
    conn = connect(root, True) if encrypted else sqlite3.connect(root / "kassiber.sqlite3")
    for wallet_id, label, config in wallets:
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,'custom',?,'2026-01-01T00:00:00Z')",
            (wallet_id, label, json.dumps(config)),
        )
    conn.commit()
    conn.close()
    return root, env


def execute(root, sql, params=(), *, encrypted=False):
    conn = connect(root, True) if encrypted else sqlite3.connect(root / "kassiber.sqlite3")
    conn.execute(sql, params)
    conn.commit()
    conn.close()


def sync(root, env, hits, url, label, *, encrypted=False):
    """One `wallets sync` compared with Python; the candidate's book stays."""
    return compare(root, env, hits, ["wallets", "sync", "--wallet", label], url, encrypted=encrypted)


def test_the_bark_wallet_books_like_python(tmp_path):
    encrypted = False
    with btcpay_server(routes(invoices=[])) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)], encrypted=encrypted)
        early, _ = sync(root, env, hits, url, "Bark", encrypted=encrypted)
        assert early["data"][0]["bark"]["linked_to_payments"] == 0, early
    # The invoices arrive later: the rows move to their payments' keys.
    with btcpay_server(routes()) as (url2, hits2):
        execute(root, "UPDATE backends SET url = ? WHERE name = 'merchant'", (url2,), encrypted=encrypted)
        later, requests = sync(root, env, hits2, url2, "Bark", encrypted=encrypted)
        assert later["data"][0]["bark"]["identities_rekeyed"] == 3, later
        again, _ = sync(root, env, hits2, url2, "Bark", encrypted=encrypted)
        assert again["data"][0]["imported"] == 0, again
    assert any("/BTC-ARK/wallet/transactions" in path for path in requests)
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


@pytest.mark.parametrize(
    "rows",
    [
        [*ROWS, _row("11-1", "teleport", "0.0001")],
        [_row("1", "refresh", "0.0001")],
        [_row("1", "ark-send", "-0.0001", fee="0.0002")],
        [_row("1", "ark-send", "-0.000000000001")],
        [{"kind": "ark-send"}],
        ["not a row"],
        [_row("5-1:a", "ark-receive", "0.0001", movementId=3, invoice="inv-old", outpoint=f"{'f1' * 32}:0"),
         _row("5-1:b", "ark-receive", "0.0001", movementId=3, invoice="inv-old", outpoint=f"{'f2' * 32}:0")],
    ],
    ids=["unknown-kind", "inflow", "fee", "sub-msat", "no-id", "not-object", "ambiguous"],
)
def test_bark_rows_refuse_or_stay_unlinked_like_python(tmp_path, rows):
    with btcpay_server(routes(rows=rows)) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)])
        sync(root, env, hits, url, "Bark")


def _ledger_book(tmp_path, url, *, encrypted=False):
    root, env = book(tmp_path, url, [("ark", "Bark payments", ARK_LEDGER)], encrypted=encrypted)
    return root, env


def test_a_ledger_switches_to_the_bark_wallet_like_python(tmp_path):
    encrypted = False
    with btcpay_server(routes(served="no")) as (url, hits):
        root, env = _ledger_book(tmp_path, url, encrypted=encrypted)
        ledger, _ = sync(root, env, hits, url, "Bark payments", encrypted=encrypted)
        assert ledger["data"][0]["bark_wallet_history"] == "not_served", ledger
    execute(root, "UPDATE transactions SET note = 'reviewed' WHERE wallet_id = 'ark'", encrypted=encrypted)
    for state, expected in (("unset", "not_served"), ("permission", "missing_permission"), ("busy", "unavailable")):
        with btcpay_server(routes(served=state)) as (url, hits):
            execute(root, "UPDATE backends SET url = ? WHERE name = 'merchant'", (url,), encrypted=encrypted)
            kept, _ = sync(root, env, hits, url, "Bark payments", encrypted=encrypted)
            assert kept["data"][0]["bark_wallet_history"] == expected, kept
    with btcpay_server(routes()) as (url, hits):
        execute(root, "UPDATE backends SET url = ? WHERE name = 'merchant'", (url,), encrypted=encrypted)
        switched, _ = sync(root, env, hits, url, "Bark payments", encrypted=encrypted)
        assert switched["data"][0]["bark"]["ledger_rows_kept"] == 3, switched
        sync(root, env, hits, url, "Bark payments", encrypted=encrypted)
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


@pytest.mark.parametrize("problem", ["unmatched", "amount", "provenance"])
def test_a_ledger_that_cannot_switch_stays_like_python(tmp_path, problem):
    invoices = [*INVOICES, _invoice("inv-lost", "BTC-ARK", f"{'99' * 32}-0", "0.0003")] if problem == "unmatched" else INVOICES
    with btcpay_server(routes(served="no", invoices=invoices)) as (url, hits):
        root, env = _ledger_book(tmp_path, url)
        sync(root, env, hits, url, "Bark payments")
    rows = [dict(row) for row in ROWS]
    if problem == "amount":
        rows[4]["amount"] = "0.00019000"
    server = routes(rows=rows, invoices=invoices)
    if problem == "provenance":
        server[STORE + "/invoices"] = (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewinvoices"})
    with btcpay_server(server) as (url, hits):
        execute(root, "UPDATE backends SET url = ? WHERE name = 'merchant'", (url,))
        refused, _ = sync(root, env, hits, url, "Bark payments")
        assert refused["kind"] == "error", refused


# A Lightning refund paid from Bark: the BTC-LN ledger booked its amount,
# the Bark wallet also knows the routing fee.
PAYOUT = {
    "id": "po-ln", "pullPaymentId": "pp-ln", "payoutMethodId": "BTC-LN", "state": "Completed", "date": 1767312000,
    "payoutAmount": "0.00003", "payoutCurrency": "BTC", "originalCurrency": "EUR", "originalAmount": "1.50",
    "paymentProof": {"proofType": "PayoutLightningBlob", "paymentHash": H_PAYOUT},
}


def _overlap_server():
    return routes(
        rows=[
            _row("21-1", "lightning-send", "-0.00003020", fee="0.00000020", paymentHash=H_PAYOUT),
            _row("20-1", "lightning-receive", "0.00030000", fee="0", paymentHash=H_BTC_LN, invoice="inv-bln"),
        ],
        invoices=[_invoice("inv-bln", "BTC-LN", H_BTC_LN, "0.0003")],
        payouts=[PAYOUT],
    )


@pytest.mark.parametrize("order", ["ledger-first", "bark-first"])
def test_lightning_settled_into_bark_is_booked_once_like_python(tmp_path, order):
    encrypted = False
    with btcpay_server(_overlap_server()) as (url, hits):
        root, env = book(tmp_path, url, [("ln", "Lightning", LN_LEDGER), ("bark", "Bark", BARK)], encrypted=encrypted)
        labels = ["Lightning", "Bark", "Lightning"] if order == "ledger-first" else ["Bark", "Lightning", "Bark"]
        outcomes = [sync(root, env, hits, url, label, encrypted=encrypted)[0]["data"][0] for label in labels]
    if order == "ledger-first":
        # The receive and the refund move, the refund taking Bark's routing fee.
        assert outcomes[1]["bark"]["lightning_ledger_rows_moved"] == 2, outcomes[1]
        assert outcomes[2]["ledger"]["held_tracked_elsewhere"] == 2, outcomes[2]
    else:
        assert outcomes[1]["ledger"]["held_tracked_elsewhere"] == 2, outcomes[1]
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


def test_a_movement_paying_two_payments_links_each_like_python(tmp_path):
    invoices = [_invoice("inv-split", "BARK", "inv-split#ark-movement-5-0", "0.0004")]
    invoices[0]["paymentMethods"][0]["payments"].append(
        {"id": "inv-split#ark-movement-5-1", "value": "0.0016", "status": "Settled", "receivedDate": 1767312000}
    )
    rows = [
        _row("5-1:a", "ark-receive", "0.0004", movementId=5, invoice="inv-split", outpoint=f"{'f1' * 32}:0"),
        _row("5-1:b", "ark-receive", "0.0016", movementId=5, invoice="inv-split", outpoint=f"{'f2' * 32}:0"),
    ]
    with btcpay_server(routes(rows=rows, invoices=invoices)) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)])
        linked, _ = sync(root, env, hits, url, "Bark")
    assert linked["data"][0]["bark"]["linked_to_payments"] == 2, linked


def test_a_bark_wallet_whose_plugin_stops_serving_fails_like_python(tmp_path):
    """No fallback to BTCPay 1.x's on-chain path: the 404 is the answer."""
    with btcpay_server(routes(served="no")) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)])
        failed, requests = sync(root, env, hits, url, "Bark")
    assert failed["kind"] == "error" or failed["data"][0]["status"] == "error", failed
    assert not any("/onchain/" in path for path in requests), requests


@pytest.mark.parametrize("blocker", ["node", "evidence"])
def test_a_lightning_payment_that_cannot_move_refuses_like_python(tmp_path, blocker):
    with btcpay_server(_overlap_server()) as (url, hits):
        root, env = book(tmp_path, url, [("ln", "Lightning", LN_LEDGER), ("bark", "Bark", BARK), ("other", "Other", {"chain": "bitcoin"})])
        if blocker == "node":
            execute(
                root,
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,raw_json,payment_hash,payment_hash_source,created_at) "
                "VALUES('node-in','w','p','other','lnd:invoice:x','fp-node','2026-01-02T00:00:00Z','inbound','BTC',30000000,0,'lnd_invoice','{}',?,'lnd','2026-01-01T00:00:00Z')",
                (H_BTC_LN,),
            )
        else:
            sync(root, env, hits, url, "Lightning")
            execute(
                root,
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,raw_json,created_at) "
                "VALUES('out','w','p','other','o','fp-o','2026-01-02T00:00:00Z','outbound','BTC',30000000,0,'withdrawal','{}','2026-01-01T00:00:00Z')",
            )
            # Source-of-funds evidence binds the ledger row to its wallet.
            execute(
                root,
                "INSERT INTO source_funds_links(id,workspace_id,profile_id,from_transaction_id,to_transaction_id,link_type,asset,created_at,updated_at) "
                "SELECT 'sof','w','p','out',id,'transfer','BTC','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z' FROM transactions "
                "WHERE wallet_id = 'ln' AND payment_hash = ?",
                (H_BTC_LN,),
            )
        refused, _ = sync(root, env, hits, url, "Bark")
        assert refused["kind"] == "error", refused


@pytest.mark.parametrize(
    "argv",
    [
        ["wallets", "sync-btcpay", "--wallet", "Cold", "--backend", "merchant", "--store-id", "shop", "--payment-method-id", "btc-ark"],
        ["wallets", "sync-btcpay", "--wallet", "Cold", "--backend", "merchant", "--store-id", "shop", "--payment-method-id", "BARK"],
        ["wallets", "attach-btcpay", "--wallet", "Cold", "--backend", "merchant", "--store-id", "shop", "--payment-method-id", "BTC-ARK"],
    ],
    ids=["sync-btcpay", "legacy-id", "attach-refused"],
)
def test_bark_cli_routes_like_python(tmp_path, argv):
    with btcpay_server(routes()) as (url, hits):
        root, env = book(tmp_path, url, [])
        compare(root, env, hits, argv, url)


def test_bark_setup_and_inspection_like_python(tmp_path):
    server = routes()
    server["/api/v1/server/info"] = (200, {"version": "2.4.7", "fullySynched": True, "supportedPaymentMethods": ["BTC-CHAIN", "BTC-ARK"]})
    server["/api/v1/api-keys/current"] = (200, {"label": "Kassiber", "permissions": ["btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet"]})
    server["/api/v1/stores"] = (200, [{"id": "shop", "name": "Shop"}, {"id": "old", "name": "Old"}])
    # The on-chain preview tells the server's network, which the Bark wallet takes.
    server[STORE + "/payment-methods"] = (200, [{"paymentMethodId": "BTC-CHAIN", "enabled": True},
                                                {"paymentMethodId": "BTC-ARK", "enabled": True}])
    server[STORE + "/payment-methods/BTC-CHAIN/wallet/preview"] = (200, {"addresses": [{"address": OWNED_A}]})
    server["/api/v1/stores/old/payment-methods"] = (200, [{"paymentMethodId": "BARK", "enabled": True}])
    with btcpay_server(server) as (url, hits):
        root, env = book(tmp_path, url, [])
        for argv in (
            ["btcpay", "inspect", "--backend", "merchant"],
            ["btcpay", "setup", "--backend", "merchant", "--label", "Shop", "--recommended", "--dry-run"],
            ["btcpay", "setup", "--backend", "merchant", "--label", "Shop", "--route", "old:BTC-ARK=existing_wallet@Cold"],
            ["btcpay", "setup", "--backend", "merchant", "--label", "Shop", "--recommended", "--no-sync"],
        ):
            last = compare_cli_masked(root, env, argv, url)
    bark = [w for w in last["data"]["wallet_sources"] if w["config"]["payment_method_id"] == "BTC-ARK"]
    assert bark and bark[0]["config"]["network"] == "main", last["data"]


# --- The native daemon's freshness jobs, on plaintext and SQLCipher books -------------------


def _jobs(root, env, hits, url, *, encrypted):
    from tests.test_rust_freshness_sources import _compare

    run_args = {"rates": False, "journals": False}
    return _compare(root, env, hits, [("ui.freshness.run", run_args), ("ui.freshness.run", run_args),
                                      ("ui.freshness.run", {**run_args, "force_full": True})], url, encrypted=encrypted)


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_bark_freshness_jobs_match_python(tmp_path, encrypted):
    """The Bark wallet's job, three runs: the later ones read the checkpoint the first stored, the last is forced."""
    with btcpay_server(routes()) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)], encrypted=encrypted)
        rows = _jobs(root, env, hits, url, encrypted=encrypted)
    completed = [row for row in rows if row.get("kind") == "ui.freshness.run"][0]["data"]["completed"]
    assert [job["status"] for job in completed] == ["done"], completed


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_a_ledger_switches_in_its_freshness_job_like_python(tmp_path, encrypted):
    """The ledger's job books payments while the plugin serves no wallet; the
    first run after it does switches, and the switch outlives the job's own
    `last_synced_at` stamp."""
    with btcpay_server(routes(served="no")) as (url, hits):
        root, env = _ledger_book(tmp_path, url, encrypted=encrypted)
        _jobs(root, env, hits, url, encrypted=encrypted)
    with btcpay_server(routes()) as (url, hits):
        execute(root, "UPDATE backends SET url = ? WHERE name = 'merchant'", (url,), encrypted=encrypted)
        rows = _jobs(root, env, hits, url, encrypted=encrypted)
    completed = [row for row in rows if row.get("kind") == "ui.freshness.run"][0]["data"]["completed"]
    assert [job["status"] for job in completed] == ["done"], completed


# --- Recorded stores (tests/fixtures/btcpay_bark/) -------------------------------------------

FIXTURES = __import__("pathlib").Path(__file__).parent / "fixtures" / "btcpay_bark"


def recorded_routes(directory):
    """A recorded regtest store as the Greenfield fake (store id `shop`)."""

    def recorded(name):
        data = json.loads((FIXTURES / directory / f"{name}.json").read_text())
        return data["status"], data["body"]

    status, rows = recorded("wallet.transactions")
    result = {
        STORE + "/invoices": _invoices(recorded("invoices")[1]),
        STORE + "/payouts": recorded("payouts"),
        STORE + "/pull-payments": (404, b""),
        STORE + "/payment-requests": (200, []),
        STORE + "/payment-methods/BTC-ARK/wallet": recorded("wallet.overview"),
        STORE + "/payment-methods/BTC-ARK/wallet/transactions": _paged(rows) if status == 200 else (status, rows),
    }
    return result


def test_a_recorded_plugin_store_books_like_python(tmp_path):
    with btcpay_server(recorded_routes("0.13.0")) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)])
        booked, _ = sync(root, env, hits, url, "Bark")
    assert booked["data"][0]["bark"]["linked_to_payments"] >= 5, booked
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


def test_a_recorded_pre_0_10_store_switches_like_python(tmp_path):
    with btcpay_server(recorded_routes("0.9.6")) as (url, hits):
        root, env = book(tmp_path, url, [("ark", "Bark payments", ARK_LEDGER), ("ln", "Lightning", LN_LEDGER)])
        sync(root, env, hits, url, "Bark payments")
        sync(root, env, hits, url, "Lightning")
    with btcpay_server(recorded_routes("0.9.6-upgraded-to-0.13.0")) as (url, hits):
        execute(root, "UPDATE backends SET url = ? WHERE name = 'merchant'", (url,))
        switched, _ = sync(root, env, hits, url, "Bark payments")
        held, _ = sync(root, env, hits, url, "Lightning")
    assert switched["data"][0]["bark"]["ledger_rows_kept"] == 1, switched
    assert switched["data"][0]["bark"]["lightning_ledger_rows_moved"] == 2, switched
    assert held["data"][0]["ledger"]["held_tracked_elsewhere"] == 3, held


@pytest.mark.parametrize("environment", ["main", "regtest"])
def test_a_recorded_store_meets_the_books_network_like_python(tmp_path, environment):
    binding = (
        "INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,domains_json,"
        "acknowledgements_json,inventory_digest,created_at) VALUES('p','env',?,1,?,'[]','[]','x','2026-01-01T00:00:00Z')"
    )
    with btcpay_server(recorded_routes("0.13.0")) as (url, hits):
        root, env = book(tmp_path, url, [("bark", "Bark", BARK)])
        instance = "00000000-0000-4000-8000-000000000001" if environment == "regtest" else None
        execute(root, binding, (environment, instance))
        outcome, _ = sync(root, env, hits, url, "Bark")
    if environment == "main":
        assert outcome["kind"] == "error" or outcome["data"][0]["code"] == "book_network_mismatch", outcome
