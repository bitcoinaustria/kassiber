"""Freeze Python's pure BTCPay normalizers for the native port.

Sections: payment-method classification, key grants, invoice origins,
payout normalization, inspection helpers and invoice/payment normalization
with the page fingerprints the incremental checkpoint compares.
"""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber import sync_btcpay
from kassiber.btcpay import discovery, origins, payment_methods, payouts, permissions
from kassiber.errors import AppError

OUTPUT = ROOT / "crates/chain/fixtures/python_btcpay.json"
SECRET = "DISCARDED_BTCPAY_SECRET"


def outcome(func, *args, **kwargs):
    try:
        return {"ok": func(*args, **kwargs)}
    except AppError as error:
        return {"error": {"code": error.code, "message": str(error), "hint": error.hint, "details": error.details}}
    except Exception as error:  # noqa: BLE001 - frozen as the reference behaves
        return {"exception": type(error).__name__}


METHOD_IDS = [
    None, "", " ", "BTC", "btc", "BTC-CHAIN", "btc-chain", "btc_chain", "BTC-OnChain", "BTC-BTCLike", "LBTC",
    "LBTC-CHAIN", "BTC-LN", "BTC-LightningNetwork", "BTC_LightningLike", "BTC-LNURL", "BTC_LNURLPAY",
    "BTC-LNURL-PAY", "XMR-CHAIN", "XMR", "USDT-CHAIN", "LBTC-LN", "BTC-Boltz", "BITCOIN", "SOMEPLUGIN",
    "BTC-", "-CHAIN", "Ä", "ÄÖÜ", "btc-chain ", " lbtc_onchain ", "ETH-LN", "BTC-L-N", "12345", "ABCDEF",
    7, "ﬀ-CHAIN", "BTC-ARK", "btc_ark", "BARK", " bark ", "BARK-CHAIN", "bark_onchain", "BARK-LN", "BARKS",
]

GRANTS = [
    None, [], ["btcpay.store.canviewstoresettings"], ["btcpay.store.canmodifystoresettings"],
    ["btcpay.store.canviewinvoices:store-a", "btcpay.store.canviewpayouts:store-a"],
    ["btcpay.store.canviewstoresettings:store-b", " btcpay.store.canviewinvoices : x ", "unrestricted"],
    ["btcpay.server.canmodifyserversettings"], ["btcpay.server.canviewusers"],
    ["btcpay.store.cancreateinvoice:s"], ["", "  ", "btcpay.store.canviewreports"],
    ["btcpay.user.canmodifyprofile", "btcpay.store.canviewstoresettings:"],
    ["custom.permission:with-colon", "btcpay.store.canviewstoresettings:s1:extra"],
    ["btcpay.store.canviewstoresettings", "btcpay.store.canviewwallet"],
    ["btcpay.store.canviewstoresettings:store-a", "btcpay.store.canviewwallet:store-b"],
    ["btcpay.store.canmanagewallets:store-a"], ["btcpay.store.cansigntransactions"],
    ["btcpay.store.canmanagewalletsettings:store-a"],
    ["btcpay.store.canmanageinvoicestatus", "btcpay.store.cansendstoreemails"],
]

# `server_version` inputs: BTCPay's reported version strings and junk.
VERSIONS = [
    None, "", " ", 0, 2, False, "2", "2.3.9", "2.4.7", "v2.4.0", " 2.4.0-beta ", "2.10", "3.0", "1.13.7",
    "2.x", "x.4", "２.４", "2_4.1", "+2.4", "vv2.4", "2.4.", ".4", "2..4", "99999999999999999999.1",
]

# Version tuples the capability tables are judged against.
WALLET_VERSIONS = [None, (2, 3), (2, 4), (1, 13), (3, 0)]

INVOICES = [
    {"id": "inv1"},
    {"id": "inv2", "orderId": "PAY_REQUEST_req9", "metadata": {"itemDesc": "  Consulting  "}},
    {"id": "inv3", "metadata": {"orderUrl": "https://shop.example/apps/AbCd/pos", "posData": {"cart": [{"title": "Tea"}, {"id": "x"}, {"title": "A"}, {"title": "B"}]}}},
    {"id": "inv4", "metadata": {"orderUrl": "https://shop.example/apps/crowdfund/xyz?x=1"}},
    {"id": "inv5", "metadata": {"orderUrl": "https://shop.example/wp-admin/post.php?post=1", "orderNumber": 42}},
    {"id": "inv6", "metadata": {"shopifyOrderId": 1, "itemDesc": "Shirt"}},
    {"id": "inv7", "metadata": {"appId": "app1", "appName": "Plugin"}},
    {"id": "inv8", "orderId": "external-1"},
    {"id": "inv9", "metadata": {"source": "Odoo"}, "orderId": "o-9"},
    {"id": "inv10", "metadata": {"paymentRequestId": "req1"}},
    {"id": "inv11", "orderId": "pos-77"},
    {"id": "inv12", "metadata": {"appName": "Crowdfund Spring", "orderUrl": "https://shop.example/apps/a%2Fb/crowdfund"}},
    {"id": "inv13", "metadata": {"posData": "string pos data"}},
    {"id": "inv14", "metadata": {"orderUrl": "http://[broken/apps/x/pos"}},
    {"id": "inv15", "metadata": {"posData": {"WooCommerce": {"id": 1}}}},
    {"id": "inv16", "metadata": {"magentoOrder": 1}},
    {"id": "inv17", "metadata": {"orderUrl": "https://x/apps/pos"}},
    {"id": "inv18", "metadata": {"orderUrl": "https://x/APPS/POS/abc/"}},
    {"id": "inv19", "metadata": {"payment_request_id": " req2 ", "itemDesc": ""}},
    {"id": "inv20", "paymentRequestId": "req3", "orderId": "PAY_REQUEST_ignored"},
]

PAYOUTS = [
    {"id": "p1", "payoutMethodId": "BTC-CHAIN", "state": "Completed", "date": 1700000000,
     "payoutAmount": "0.001", "payoutCurrency": "BTC", "originalCurrency": "EUR", "originalAmount": "30",
     "paymentProof": {"proofType": "PayoutTransactionOnChainBlob", "transactionId": "AB" * 32, "candidates": ["x"]},
     "destination": "bc1qdestination", "pullPaymentId": "pp1"},
    {"id": "p2", "paymentMethod": "BTC-LightningNetwork", "state": "Completed", "date": "1700000001",
     "paymentProof": {"ProofType": "PayoutLightningBlob", "PaymentHash": "cd" * 32, "Preimage": SECRET},
     "destination": "lnbc1" + "x" * 20, "pullPaymentId": "pp2"},
    {"id": "p3", "payoutMethodId": "BTC_LNURLPAY", "state": "InProgress", "date": "2024-01-01T00:00:00Z",
     "destination": "LIGHTNING:LNURL1ABC", "metadata": {"source": "Prism"}},
    {"id": "p4", "payoutMethodId": "LBTC-CHAIN", "payoutAmount": "5", "payoutCurrency": "USDT",
     "originalCurrency": "SATS", "originalAmount": "100", "pullPaymentId": "missing"},
    {"id": "", "payoutMethodId": "BTC-CHAIN"},
    {"id": "p6", "date": "bad", "paymentProof": "not a mapping", "metadata": {"label": "Payroll"}},
    {"id": "p7", "date": -1, "paymentProof": {"txid": "not-hex", "id": "ef" * 32}, "destination": "user@example.com"},
    {"id": "p8", "cryptoCode": "BTC", "paymentMethodAmount": "0.5", "currency": "USD", "amount": "20000"},
]

PULL_PAYMENTS = {
    "pp1": {"id": "pp1", "name": "Refund 5ZbLqJLTwm9ZbDtZzkB3Yp", "viewLink": "https://shop/pull-payments/pp1"},
    "pp2": {"id": "pp2", "name": "Refund customers", "description": "refund 5ZbLqJLTwm9ZbDtZzkB3Yq"},
}

ADDRESSES = ["bcrt1qx", "BC1QABC", "tb1q", "tlq1q", "tex1q", "lq1q", "ex1q", "el1q", "ert1q", "1Legacy", "", None, " bc1 "]

PAYMENT_METHOD_ROWS = [
    [{"paymentMethodId": "BTC-CHAIN", "enabled": True, "name": "Bitcoin"}, {"paymentMethod": "BTC-LN"},
     {"cryptoCode": "LBTC", "paymentType": "BTCLike_OnChain"}, {"cryptoCode": "BTC", "paymentType": "LightningLike"},
     {"enabled": False}, "skip", {"id": "XMR-CHAIN", "label": "Monero", "enabled": 0}],
    {"BTC": True, "BTC-LightningNetwork": {"enabled": False, "name": "LN"}, "XMR": 0},
    None, "text",
]

WALLET_PAGES = [
    [],
    [{"transactionHash": "a" * 64, "timestamp": 1700000000, "amount": "0.1", "confirmations": 3, "status": "Confirmed",
      "comment": "Note", "labels": {"label": {"type": "raw", "text": "label"}}},
     {"transactionId": "b", "timestamp": 1700000001, "amount": "-0.2", "labels": ["x", "y"]},
     {"id": "c", "comment": None}, "not a dict"],
]

INVOICE_PAGES = [
    [],
    [{"id": "i1", "status": "Settled", "metadata": {"orderId": "o1", "orderUrl": "u"},
      "paymentMethods": [{"paymentMethodId": "BTC-CHAIN", "rate": "50000", "destination": "bc1q",
                          "payments": [{"id": "e" * 64 + "-0", "value": "0.001", "receivedDate": 1700000000, "status": "Settled"}]}]},
     {"id": "i2", "status": "New", "additionalStatus": "None"},
     {"invoiceId": "i3", "status": "Processing", "paidAmount": "1"},
     {"status": "Expired"}, "not a dict"],
]

RAW_PAYMENTS = [
    {"id": "f" * 64, "paymentMethodId": "BTC-CHAIN", "value": "0.1"},
    {"id": "f" * 64, "paymentMethodId": "BTC-LN", "value": "0.1", "receivedDate": "2024-01-01T00:00:00Z"},
    {"paymentId": "a" * 64 + "-3", "details": {"paymentMethod": "LBTC", "transactionHash": "b" * 64, "fee": "0.0001"},
     "receivedTime": 1700000000.9, "cryptoAmount": 0},
    {"accountedPaymentId": "x", "paymentMethodData": "BTC_LNURLPAY", "preimageHash": "c" * 64, "address": "addr"},
    {"transactionId": "t", "rate": 1.5, "destination": "d", "status": "Invalid", "createdTime": "12"},
]


BARK_PAGES = [
    [],
    [{"id": "b", "kind": "refresh", "amount": "-0.0000005", "fee": "0.0000005", "labels": {"x": {"type": "raw", "text": "x"}}},
     {"id": " a ", "kind": "ark-receive", "amount": 0.0001, "outpoint": "AB:0", "transactionHash": None, "timestamp": 1}],
    [{"id": 7, "paymentHash": "cd" * 32, "status": "Unconfirmed", "confirmations": 0}],
    [{"id": "a"}, "not a row"],
    [{"kind": "ark-send"}],
    [{"id": "r\ud800"}],
]


def render():
    sections = {
        "payment_methods": [{
            "input": value,
            "canonical": outcome(payment_methods.canonical_payment_method_id, value),
            "split": outcome(payment_methods.split_payment_method_id, value),
            "classify": outcome(payment_methods.classify_payment_method, value),
            "wallet_history": outcome(payment_methods.is_wallet_history_payment_method, value),
            "require": outcome(payment_methods.require_wallet_history_payment_method, value),
            "asset": outcome(payment_methods.asset_for_payment_method, value),
        } for value in METHOD_IDS],
        "grants": [{
            "input": value,
            "summary": permissions.grant_summary(permissions.parse_key_grant(value)),
            "store_a": permissions.parse_key_grant(value).capabilities("store-a"),
            "excess": permissions.excess_permissions(permissions.parse_key_grant(value)),
            "by_version": [{
                "version": version,
                "summary": permissions.grant_summary(permissions.parse_key_grant(value), version),
                "store_a": permissions.parse_key_grant(value).capabilities("store-a", version),
            } for version in WALLET_VERSIONS],
        } for value in GRANTS],
        "versions": [{
            "input": value,
            "version": permissions.server_version(value),
        } for value in VERSIONS],
        "wallet_versions": [{
            "version": version,
            "tables": [list(table.items()) for table in permissions.capability_permissions(version)],
            "history_preset": permissions.wallet_history_preset(version),
        } for version in WALLET_VERSIONS],
        "guides": [{
            "server": server,
            "preset": preset,
            "scope": scope,
            "guide": outcome(permissions.key_setup_guide, server, preset=preset, store_scope=scope),
            "scoped_url": outcome(permissions.build_authorize_url, server or "https://shop.example",
                                  preset=preset, store_ids=["s1", " ", "s 2"], store_scope=scope),
        } for server in [None, "https://shop.example/"] for preset in ["read_only", "wallet_history", "nope"]
            for scope in [None, "all", "single"]],
        "labels": {value: permissions.permission_label(value) for value in [
            "btcpay.store.canviewinvoices:x", "unrestricted", "other", "", "btcpay.store.canviewreports",
            "btcpay.store.canviewwallet:s1"]},
        "origins": [{
            "input": invoice,
            "normalized": outcome(sync_btcpay._normalize_invoice_provenance, "store", invoice,
                                  payment_request_titles={"req1": "Title one", "req9": "Ninth"}),
        } for invoice in INVOICES],
        "payouts": [{
            "input": payout,
            "normalized": outcome(payouts.normalize_payout, "store", payout, pull_payments=PULL_PAYMENTS),
        } for payout in PAYOUTS],
        "addresses": [{"input": value, "network": list(discovery.network_from_address(value))} for value in ADDRESSES],
        "fingerprints": [discovery.wallet_fingerprint(addresses) for addresses in
                         [[], ["b", "a", "a", " "], ["bc1qa", "bc1qb", "bc1qc"]]],
        "payment_method_rows": [{
            "input": rows,
            "normalized": [discovery.normalize_payment_method("store", row)
                           for row in discovery._payment_method_rows(rows)],
        } for rows in PAYMENT_METHOD_ROWS],
        "bark_pages": [{
            "input": page,
            "stable_ids": outcome(sync_btcpay._bark_page_stable_ids, page),
            "fingerprint": outcome(sync_btcpay._bark_page_fingerprint, page),
        } for page in BARK_PAGES],
        "wallet_pages": [{
            "input": page,
            "stable_ids": outcome(sync_btcpay._page_stable_ids, page),
            "fingerprint": outcome(sync_btcpay._page_fingerprint, page),
        } for page in WALLET_PAGES],
        "invoice_pages": [{
            "input": page,
            "stable_ids": outcome(sync_btcpay._invoice_page_stable_ids, page),
            "fingerprint": outcome(sync_btcpay._invoice_page_fingerprint, page),
            "open": outcome(sync_btcpay._invoice_page_has_open_invoice, page),
        } for page in INVOICE_PAGES],
        "payments": [{
            "input": payment,
            "normalized": outcome(sync_btcpay._normalize_invoice_payment, {"currency": "EUR", "amount": "10"}, payment),
            "from_methods": outcome(sync_btcpay._payments_from_invoice_payment_methods,
                                    [{"paymentMethodId": "btc", "rate": "1", "payments": [payment, "skip"]}, "skip",
                                     {"paymentMethod": "BTC-LN", "payments": "bad"}]),
        } for payment in RAW_PAYMENTS],
    }
    return json.dumps(sections, ensure_ascii=True, indent=1, sort_keys=False) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    rendered = render()
    if parser.parse_args().check:
        if OUTPUT.read_text() != rendered:
            raise SystemExit("BTCPay normalizer vectors differ from Python")
    else:
        OUTPUT.write_text(rendered)
    print(f"BTCPay normalizers: {sum(len(v) for v in json.loads(rendered).values())} reference cases")


if __name__ == "__main__":
    main()
