"""BTCPay Greenfield client, permission model, catalog, inspection, and setup plan."""

import io
import json
import sqlite3
import tempfile
import unittest
from email.message import Message
from pathlib import Path
from urllib import error as urlerror
from urllib.parse import parse_qs, urlsplit

from kassiber.btcpay.client import (
    GreenfieldClient,
    normalize_server_url,
    scrub_api_key_payload,
    server_url_identity,
)
from kassiber.btcpay.discovery import inspect_btcpay_connection, network_from_address, wallet_fingerprint
from kassiber.btcpay.origins import classify_invoice_origin
from kassiber.btcpay.payment_methods import (
    canonical_payment_method_id,
    classify_payment_method,
    require_wallet_history_payment_method,
)
from kassiber.btcpay.payouts import normalize_payout
from kassiber.btcpay.permissions import (
    build_authorize_url,
    excess_permissions,
    key_risk,
    key_setup_guide,
    parse_key_grant,
)
from kassiber.core import btcpay_setup
from kassiber.core import commercial
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.sync_btcpay import (
    fetch_btcpay_invoice_provenance,
    fetch_btcpay_payouts,
    fetch_btcpay_records,
)
from kassiber.time_utils import now_iso
from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan

# A regtest store wallet (public key material only).
STORE_TPUB = (
    "tpubDDFb1TsxwCpVxT4hisQ8UK4HVnMMcNXToye6Ce6Buo76BSVtfGmVLv6EGqigggNbrLzfc"
    "mVYijtFZ1xWCvPLQjCP9Zk3QTFLzpHvvpTqr5Y"
)
STORE_DESCRIPTOR = f"wpkh([ffd9c4c0/84h/1h/0h]{STORE_TPUB}/0/*)"
STORE_CHANGE_DESCRIPTOR = f"wpkh([ffd9c4c0/84h/1h/0h]{STORE_TPUB}/1/*)"


class _Response:
    def __init__(self, payload):
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self):
        return json.dumps(self.payload).encode("utf-8")


def _http_error(url, status, body=None):
    raw = b"" if body is None else json.dumps(body).encode("utf-8")
    return urlerror.HTTPError(url, status, "error", Message(), io.BytesIO(raw))


class _RoutedOpener:
    """Fake Greenfield server keyed by exact path (query ignored)."""

    def __init__(self, routes):
        self.routes = routes
        self.urls = []

    def open(self, request, timeout=None):
        del timeout
        url = request.full_url
        self.urls.append(url)
        path = urlsplit(url).path
        handler = self.routes.get(path)
        if handler is None:
            raise _http_error(url, 404)
        if callable(handler):
            handler = handler(url)
        if isinstance(handler, tuple):
            status, body = handler
            raise _http_error(url, status, body)
        return _Response(handler)


def _client(opener, url="https://pay.example.com"):
    return GreenfieldClient(base_url=url, token="secret-key", timeout=5, opener=opener)


def _preview(addresses):
    return {"addresses": [{"address": value, "keyPath": f"0/{i}", "index": i} for i, value in enumerate(addresses)]}


def _store_addresses(count=5):
    plan = load_descriptor_plan(
        {
            "descriptor": STORE_DESCRIPTOR,
            "change_descriptor": STORE_CHANGE_DESCRIPTOR,
            "chain": "bitcoin",
            "network": "regtest",
        }
    )
    return [target.address for target in derive_descriptor_targets(plan, 0, 0, count)]


class ClientAndPermissionTest(unittest.TestCase):
    def test_server_url_normalization_accepts_pasted_browser_urls(self):
        cases = {
            "https://pay.example.com/": "https://pay.example.com",
            "https://pay.example.com/api/v1/": "https://pay.example.com",
            "https://pay.example.com/stores/AbC/invoices": "https://pay.example.com",
            "  https://example.com/btcpay/api-keys  ": "https://example.com/btcpay",
            "HTTP://127.0.0.1:23001": "http://127.0.0.1:23001",
        }
        for raw, expected in cases.items():
            with self.subTest(raw=raw):
                self.assertEqual(normalize_server_url(raw), expected)
        for raw in ("pay.example.com", "ftp://pay.example.com", "https://user:pw@pay.example.com"):
            with self.subTest(raw=raw), self.assertRaises(AppError) as ctx:
                normalize_server_url(raw)
            self.assertEqual(ctx.exception.code, "validation")
        self.assertEqual(
            server_url_identity("https://PAY.example.com:443/api/v1"),
            server_url_identity("https://pay.example.com"),
        )

    def test_missing_permission_error_names_the_permission(self):
        opener = _RoutedOpener(
            {
                "/api/v1/stores/S1/invoices": (
                    403,
                    {"code": "missing-permission", "missingPermission": "btcpay.store.canviewinvoices"},
                )
            }
        )
        with self.assertRaises(AppError) as ctx:
            _client(opener).get_json("/api/v1/stores/S1/invoices", context="invoices")
        self.assertEqual(ctx.exception.code, "auth_error")
        self.assertIn("View invoices", str(ctx.exception))
        self.assertEqual(ctx.exception.details["missing_permission"], "btcpay.store.canviewinvoices")

    def test_rejected_key_is_an_auth_error(self):
        opener = _RoutedOpener({"/api/v1/stores": (401, {"code": "unauthenticated"})})
        with self.assertRaises(AppError) as ctx:
            _client(opener).get_json("/api/v1/stores")
        self.assertEqual(ctx.exception.code, "auth_error")
        self.assertIn("HTTP 401", str(ctx.exception))

    def test_current_key_payload_never_keeps_the_secret(self):
        scrubbed = scrub_api_key_payload(
            {"apiKey": "secret-key", "label": "Kassiber", "permissions": ["btcpay.store.canviewstoresettings"]}
        )
        self.assertEqual(scrubbed, {"label": "Kassiber", "permissions": ["btcpay.store.canviewstoresettings"]})

    def test_view_store_settings_is_a_complete_read_only_grant(self):
        grant = parse_key_grant(["btcpay.store.canviewstoresettings:S1"])
        self.assertEqual(
            grant.capabilities("S1"),
            {
                "store_catalog": True,
                "wallet_preview": True,
                "invoices": True,
                "payment_requests": True,
                "payouts": True,
                "pull_payments": True,
                "wallet_history": False,
            },
        )
        self.assertFalse(any(grant.capabilities("S2").values()))
        self.assertEqual(key_risk(grant), "read_only")
        self.assertEqual(excess_permissions(grant), [])

    def test_key_risk_and_excess_permissions(self):
        modify = parse_key_grant(["btcpay.store.canmodifystoresettings"])
        self.assertTrue(modify.capabilities("any")["wallet_history"])
        self.assertEqual(key_risk(modify), "can_modify_store")
        self.assertEqual(excess_permissions(modify, wallet_history=True), [])
        writer = parse_key_grant(["btcpay.store.canviewinvoices:S1", "btcpay.store.cancreateinvoice:S1"])
        self.assertEqual(key_risk(writer), "can_write")
        self.assertEqual(excess_permissions(writer), ["btcpay.store.cancreateinvoice"])
        self.assertEqual(key_risk(parse_key_grant(["unrestricted"])), "server_admin")
        self.assertEqual(key_risk(parse_key_grant(None)), "unknown")

    def test_authorize_url_prefills_least_privilege_permissions(self):
        url = build_authorize_url("https://pay.example.com/api/v1")
        parts = urlsplit(url)
        query = parse_qs(parts.query)
        self.assertEqual(f"{parts.scheme}://{parts.netloc}{parts.path}", "https://pay.example.com/api-keys/authorize")
        self.assertEqual(query["permissions"], ["btcpay.store.canviewstoresettings"])
        self.assertEqual(query["strict"], ["true"])
        # A read-only key covers every store unless the user asks for one store.
        self.assertEqual(query["selectiveStores"], ["false"])
        single = parse_qs(urlsplit(build_authorize_url("https://pay.example.com", store_scope="single")).query)
        self.assertEqual(single["selectiveStores"], ["true"])
        history = parse_qs(urlsplit(build_authorize_url("https://pay.example.com", preset="wallet_history")).query)
        self.assertEqual(history["selectiveStores"], ["true"])
        with self.assertRaises(AppError):
            build_authorize_url("https://pay.example.com", store_scope="some")
        scoped = parse_qs(urlsplit(build_authorize_url("https://pay.example.com", store_ids=["S1", "S2"])).query)
        self.assertEqual(
            scoped["permissions"],
            ["btcpay.store.canviewstoresettings:S1", "btcpay.store.canviewstoresettings:S2"],
        )
        self.assertEqual(scoped["selectiveStores"], ["false"])
        guide = key_setup_guide(None, preset="wallet_history")
        self.assertEqual(guide["permissions"], ["btcpay.store.canmodifystoresettings"])
        self.assertEqual(guide["store_scope"], "single")
        self.assertNotIn("authorize_url", guide)
        with self.assertRaises(AppError):
            key_setup_guide(None, preset="everything")


class CatalogOriginAndPayoutTest(unittest.TestCase):
    def test_payment_method_catalog_covers_legacy_ids_and_plugins(self):
        self.assertEqual(canonical_payment_method_id("BTC"), "BTC-CHAIN")
        self.assertEqual(canonical_payment_method_id("btc-lightningnetwork"), "BTC-LN")
        self.assertEqual(canonical_payment_method_id("BTC_LNURLPAY"), "BTC-LNURL")
        self.assertEqual(canonical_payment_method_id("ARKADE"), "ARKADE")
        expected = {
            "BTC-CHAIN": ("onchain", "bitcoin", True, "btc_onchain"),
            "LBTC-CHAIN": ("onchain", "liquid", True, "liquid_onchain"),
            "BTC-LN": ("lightning", "bitcoin", False, "lightning"),
            "BTC-LNURL": ("lnurl", "bitcoin", False, "lnurl"),
            "USDT-CHAIN": ("onchain", None, False, "non_bitcoin"),
            "XMR-CHAIN": ("onchain", None, False, "non_bitcoin"),
            "BTC-ARK": ("plugin", "bitcoin", False, "plugin"),
        }
        for method_id, (rail, chain, history, settlement) in expected.items():
            with self.subTest(method_id=method_id):
                info = classify_payment_method(method_id)
                self.assertEqual(
                    (info["rail"], info["chain"], info["wallet_history_supported"], info["settlement"]),
                    (rail, chain, history, settlement),
                )
        with self.assertRaises(AppError) as ctx:
            require_wallet_history_payment_method("BTC-LN")
        self.assertIn("Lightning", ctx.exception.hint)

    def test_origin_classifier_reads_real_btcpay_app_metadata(self):
        pos = classify_invoice_origin(
            {},
            {
                "orderId": "N3Tboh",
                "orderUrl": "https://pay.example.com/apps/22DwNjQ4/pos",
                "itemDesc": "Espresso",
                "posData": {"cart": [{"id": "espresso", "title": None}]},
            },
            "N3Tboh",
        )
        self.assertEqual((pos["kind"], pos["app_id"], pos["label"]), ("pos", "22DwNjQ4", "Espresso"))
        crowdfund = classify_invoice_origin(
            {"type": "TopUp"},
            {"orderUrl": "https://pay.example.com/apps/2HA7kh/crowdfund", "itemDesc": "Probe Fund"},
            "3Qtxu8",
        )
        self.assertEqual((crowdfund["kind"], crowdfund["app_id"]), ("crowdfund", "2HA7kh"))
        request = classify_invoice_origin(
            {},
            {"orderId": "PAY_REQUEST_61f0", "paymentRequestId": "61f0"},
            "PAY_REQUEST_61f0",
            payment_request_titles={"61f0": "March consulting"},
        )
        self.assertEqual((request["kind"], request["label"]), ("payment_request", "March consulting"))
        from_order_only = classify_invoice_origin({}, {}, "PAY_REQUEST_abc")
        self.assertEqual(from_order_only["kind"], "payment_request")

    def test_origin_classifier_recognises_ecommerce_plugins(self):
        woo = classify_invoice_origin(
            {},
            {
                "orderNumber": "1042",
                "orderUrl": "https://shop.example/wp-admin/post.php?post=1042&action=edit",
                "posData": {"WooCommerce": {"Order ID": 1042}},
            },
            "1042",
        )
        self.assertEqual((woo["kind"], woo["source"], woo["label"]), ("ecommerce", "woocommerce", "Order 1042"))
        shopify = classify_invoice_origin({}, {"shopifyOrderId": "5512"}, "#1001")
        self.assertEqual((shopify["kind"], shopify["source"]), ("ecommerce", "shopify"))

    def test_payout_normalization_links_refunds_and_drops_lightning_secrets(self):
        pull_payments = {
            "pp-refund": {"id": "pp-refund", "name": "Refund 7yRWXHuDJaEyDXnrg8WJab", "viewLink": "https://pay.example.com/pull-payments/pp-refund"},
            "pp-split": {"id": "pp-split", "name": "Team split"},
        }
        txid = "ab" * 32
        onchain = normalize_payout(
            "S1",
            {
                "id": "po-1",
                "pullPaymentId": "pp-refund",
                "payoutMethodId": "BTC-CHAIN",
                "destination": "bcrt1qdest",
                "originalAmount": "5",
                "originalCurrency": "EUR",
                "payoutCurrency": "BTC",
                "payoutAmount": "0.0001",
                "state": "Completed",
                "date": 1790683659,
                "paymentProof": {"proofType": "PayoutTransactionOnChainBlob", "transactionId": txid},
            },
            pull_payments=pull_payments,
        )
        self.assertEqual(onchain["origin_kind"], "refund")
        self.assertEqual(onchain["invoice_id"], "7yRWXHuDJaEyDXnrg8WJab")
        self.assertEqual(onchain["txid"], txid)
        self.assertEqual((onchain["fiat_currency"], onchain["fiat_value"]), ("EUR", "5"))
        # Exact proof shape BTCPay 2.3.9 stores after paying a payout.
        live = normalize_payout(
            "S1",
            {
                "id": "po-live",
                "payoutMethodId": "BTC-CHAIN",
                "state": "Completed",
                "paymentProof": {"Accounted": True, "ProofType": "PayoutTransactionOnChainBlob", "TransactionId": txid.upper()},
            },
        )
        self.assertEqual(live["txid"], txid)
        payment_hash = "cd" * 32
        lightning = normalize_payout(
            "S1",
            {
                "id": "po-2",
                "pullPaymentId": "pp-split",
                "payoutMethodId": "BTC-LN",
                "destination": "lnbcrt10u1pjexample",
                "originalCurrency": "BTC",
                "payoutAmount": "0.00001",
                "state": "Completed",
                "paymentProof": {"proofType": "PayoutLightningBlob", "id": payment_hash, "preimage": "ef" * 32},
            },
            pull_payments=pull_payments,
        )
        self.assertEqual((lightning["origin_kind"], lightning["payment_hash"], lightning["txid"]), ("pull_payment", payment_hash, None))
        self.assertIsNone(lightning["destination"])
        self.assertIsNone(lightning["payout"]["destination"])
        self.assertNotIn("preimage", lightning["payout"]["paymentProof"])
        self.assertIsNone(lightning["fiat_currency"])
        custom = normalize_payout(
            "S1",
            {"id": "po-4", "pullPaymentId": "pp-custom", "payoutMethodId": "BTC-CHAIN"},
            pull_payments={"pp-custom": {"id": "pp-custom", "name": "Refund customers"}},
        )
        self.assertEqual((custom["origin_kind"], custom["invoice_id"]), ("pull_payment", None))
        store_payout = normalize_payout("S1", {"id": "po-3", "payoutMethodId": "BTC-CHAIN", "metadata": {"source": "Prism"}})
        self.assertEqual((store_payout["origin_kind"], store_payout["origin_label"]), ("store_payout", "Prism"))


class InspectionTest(unittest.TestCase):
    def _server(self, *, key_permissions, stores_status=200, preview=None):
        addresses = preview if preview is not None else _store_addresses()
        routes = {
            "/api/v1/server/info": {"version": "2.3.9", "fullySynched": True, "supportedPaymentMethods": ["BTC-CHAIN", "BTC-LN"]},
            "/api/v1/api-keys/current": {"apiKey": "secret-key", "label": "Kassiber", "permissions": key_permissions},
            "/api/v1/stores": (
                [
                    {"id": "S1", "name": "Main Store", "defaultCurrency": "EUR"},
                    {"id": "S2", "name": "Pop-up", "defaultCurrency": "EUR"},
                    {"id": "S3", "name": "Old shop", "archived": True},
                ]
                if stores_status == 200
                else (stores_status, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewstoresettings"})
            ),
            "/api/v1/stores/S1/payment-methods": [
                {"paymentMethodId": "BTC-CHAIN", "enabled": True},
                {"paymentMethodId": "BTC-LN", "enabled": True},
                {"paymentMethodId": "XMR-CHAIN", "enabled": True},
            ],
            # BTCPay 1.x shape: a dict keyed by payment method id.
            "/api/v1/stores/S2/payment-methods": {"BTC": {"enabled": True, "cryptoCode": "BTC"}},
            "/api/v1/stores/S1/payment-methods/BTC-CHAIN/wallet/preview": _preview(addresses),
            "/api/v1/stores/S2/payment-methods/BTC-CHAIN/wallet/preview": _preview(addresses),
        }
        return _RoutedOpener(routes)

    def test_read_only_key_inspection_detects_shared_wallets_and_network(self):
        opener = self._server(key_permissions=["btcpay.store.canviewstoresettings"])
        result = inspect_btcpay_connection(_client(opener), max_workers=1)
        public = result.public

        self.assertEqual(public["server"]["version"], "2.3.9")
        self.assertEqual(public["api_key"]["risk"], "read_only")
        self.assertEqual(public["api_key"]["scope"], "all_stores")
        self.assertEqual([store["id"] for store in public["stores"]], ["S1", "S2"])
        methods = {(m["store_id"], m["payment_method_id"]): m for m in public["payment_methods"]}
        self.assertEqual(set(methods), {("S1", "BTC-CHAIN"), ("S1", "BTC-LN"), ("S1", "XMR-CHAIN"), ("S2", "BTC-CHAIN")})
        self.assertEqual(methods[("S1", "BTC-CHAIN")]["network"], "regtest")
        self.assertFalse(methods[("S1", "BTC-CHAIN")]["wallet_history_available"])
        self.assertIsNone(methods[("S1", "BTC-LN")]["wallet_fingerprint"])
        self.assertEqual(
            methods[("S1", "BTC-CHAIN")]["wallet_fingerprint"],
            methods[("S2", "BTC-CHAIN")]["wallet_fingerprint"],
        )
        self.assertEqual(len(public["shared_wallets"]), 1)
        self.assertIn("shared_store_wallet", [warning["code"] for warning in public["warnings"]])
        serialized = json.dumps(public)
        self.assertNotIn("secret-key", serialized)
        self.assertNotIn(_store_addresses(1)[0], serialized)
        self.assertEqual(len(result.private["previews"]["S1"]["BTC-CHAIN"]), 5)

    def test_invoice_only_key_falls_back_to_scoped_store_ids(self):
        opener = self._server(
            key_permissions=["btcpay.store.canviewinvoices:S1"],
            stores_status=403,
        )
        public = inspect_btcpay_connection(_client(opener), max_workers=1).public

        self.assertEqual([store["id"] for store in public["stores"]], ["S1"])
        self.assertFalse(public["stores"][0]["capabilities"]["store_catalog"])
        self.assertTrue(public["stores"][0]["capabilities"]["invoices"])
        self.assertIn("btcpay.store.canviewstoresettings", public["stores"][0]["missing_permissions"])
        self.assertEqual(public["payment_methods"], [])
        self.assertIn("store_catalog_unavailable", [w["code"] for w in public["warnings"]])
        self.assertFalse(any("/payment-methods" in url for url in opener.urls))

    def test_rejected_key_fails_fast_and_http_transport_is_flagged(self):
        opener = _RoutedOpener({"/api/v1/server/info": (401, {"code": "unauthenticated"})})
        with self.assertRaises(AppError) as ctx:
            inspect_btcpay_connection(_client(opener))
        self.assertEqual(ctx.exception.code, "auth_error")
        insecure = self._server(key_permissions=["btcpay.store.canviewstoresettings"])
        public = inspect_btcpay_connection(_client(insecure, url="http://pay.example.com"), max_workers=1).public
        self.assertIn("unencrypted_transport", [w["code"] for w in public["warnings"]])

    def test_network_and_fingerprint_helpers(self):
        self.assertEqual(network_from_address("bc1qexample"), ("bitcoin", "main"))
        self.assertEqual(network_from_address("tb1qexample"), ("bitcoin", "test"))
        self.assertEqual(network_from_address("lq1qqexample"), ("liquid", "liquidv1"))
        self.assertEqual(network_from_address("1LegacyAddress"), (None, None))
        self.assertEqual(wallet_fingerprint(["b", "a"]), wallet_fingerprint(["a", "b", "a"]))
        self.assertIsNone(wallet_fingerprint([]))


class FetchTest(unittest.TestCase):
    backend = {"name": "btcpay", "kind": "btcpay", "url": "https://pay.example.com", "token": "secret-key"}

    def test_invoice_fetch_skips_unpaid_invoices_and_rechecks_open_ones(self):
        hydrated = []

        def payment_methods(url):
            hydrated.append(urlsplit(url).path.split("/")[-2])
            return [{"paymentMethodId": "BTC-CHAIN", "payments": [{"id": f"{'aa' * 32}-0", "value": "0.001"}]}]

        page = [
            {"id": "inv-open", "status": "Processing", "additionalStatus": "None"},
            {"id": "inv-expired", "status": "Expired", "additionalStatus": "None"},
            {"id": "inv-late", "status": "Expired", "additionalStatus": "PaidLate"},
            {"id": "inv-new", "status": "New", "additionalStatus": "None"},
        ]
        routes = {
            "/api/v1/stores/S1/invoices": lambda url: page if parse_qs(urlsplit(url).query)["skip"] == ["0"] else [],
        }
        for invoice in page:
            routes[f"/api/v1/stores/S1/invoices/{invoice['id']}/payment-methods"] = payment_methods
        metadata = {}
        fetch_btcpay_invoice_provenance(
            self.backend, "S1", page_size=4, opener=_RoutedOpener(routes), metadata=metadata
        )
        self.assertEqual(sorted(hydrated), ["inv-late", "inv-open"])

        hydrated.clear()
        second = {}
        records = fetch_btcpay_invoice_provenance(
            self.backend,
            "S1",
            page_size=4,
            opener=_RoutedOpener(routes),
            checkpoint={"btcpay_invoice_pages": metadata["btcpay_invoice_pages"]},
            metadata=second,
        )
        # The page is unchanged but still holds a Processing invoice.
        self.assertEqual(second["changed_pages"], [0])
        self.assertEqual(len(records), 4)

    def test_payment_request_titles_load_once_and_only_when_needed(self):
        page = [
            {"id": "inv-1", "status": "Settled", "metadata": {"orderId": "PAY_REQUEST_pr-1", "paymentRequestId": "pr-1"}, "paymentMethods": []},
            {"id": "inv-2", "status": "Settled", "metadata": {"paymentRequestId": "pr-2", "itemDesc": "Has a label"}, "paymentMethods": []},
        ]
        opener = _RoutedOpener(
            {
                "/api/v1/stores/S1/invoices": lambda url: page if parse_qs(urlsplit(url).query)["skip"] == ["0"] else [],
                "/api/v1/stores/S1/payment-requests": [{"id": "pr-1", "title": "Membership"}],
            }
        )
        records = fetch_btcpay_invoice_provenance(self.backend, "S1", page_size=10, opener=opener)
        self.assertEqual([record["origin_label"] for record in records], ["Membership", "Has a label"])
        self.assertEqual(sum("/payment-requests" in url for url in opener.urls), 1)

    def test_wallet_history_falls_back_to_btcpay_1x_paths(self):
        row = {"transactionHash": "ab" * 32, "amount": "0.001", "timestamp": 1_700_000_000, "confirmations": 3}
        opener = _RoutedOpener(
            {"/api/v1/stores/S1/payment-methods/onchain/BTC/wallet/transactions": lambda url: [row] if "skip=0" in url else []}
        )
        metadata = {}
        records = fetch_btcpay_records(self.backend, "S1", page_size=10, opener=opener, metadata=metadata)
        self.assertEqual(len(records), 1)
        self.assertTrue(metadata["btcpay_pagination"]["legacy_wallet_paths"])
        self.assertIn("/payment-methods/BTC-CHAIN/wallet/transactions", opener.urls[0])

    def test_payouts_without_permission_degrade_quietly(self):
        opener = _RoutedOpener(
            {"/api/v1/stores/S1/payouts": (403, {"code": "missing-permission", "missingPermission": "btcpay.store.canviewpayouts"})}
        )
        metadata = {}
        self.assertEqual(fetch_btcpay_payouts(self.backend, "S1", opener=opener, metadata=metadata), [])
        self.assertTrue(metadata["payouts_permission_missing"])


class SetupPlanTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-btcpay-plan-")
        self.conn = open_db(str(Path(self.tmp.name) / "data"))
        self.conn.row_factory = sqlite3.Row
        now = now_iso()
        self.conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Shop', ?)", (now,))
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at) VALUES('prof', 'ws', 'Main', 'EUR', ?)",
            (now,),
        )
        self.conn.execute(
            "INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at) "
            "VALUES('acct', 'ws', 'prof', 'treasury', 'Treasury', 'asset', ?)",
            (now,),
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at) "
            "VALUES('trezor', 'ws', 'prof', 'acct', 'Shop Trezor', 'descriptor', ?, ?)",
            (
                json.dumps(
                    {
                        "descriptor": STORE_DESCRIPTOR,
                        "change_descriptor": STORE_CHANGE_DESCRIPTOR,
                        "chain": "bitcoin",
                        "network": "regtest",
                    }
                ),
                now,
            ),
        )
        self.conn.commit()
        self.runtime_config = {
            "backends": {
                "shop-ro": {"name": "shop-ro", "kind": "btcpay", "url": "https://pay.example.com/"},
                "cafe-history": {"name": "cafe-history", "kind": "btcpay", "url": "https://PAY.example.com/api/v1"},
            }
        }

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def _inspection(self, key_permissions):
        opener = InspectionTest()._server(key_permissions=key_permissions)
        return inspect_btcpay_connection(_client(opener), max_workers=1)

    def _plan(self, key_permissions, backend="shop-ro"):
        return btcpay_setup.plan_btcpay_setup(
            self.conn,
            "prof",
            self._inspection(key_permissions),
            backend_name=backend,
            runtime_config=self.runtime_config,
            server_url="https://pay.example.com",
        )

    def test_plan_recognises_the_tracked_wallet_for_every_store_sharing_it(self):
        plan = self._plan(["btcpay.store.canviewstoresettings"])
        methods = {(m["store_id"], m["payment_method_id"]): m for m in plan["payment_methods"]}

        for store_id in ("S1", "S2"):
            recommendation = methods[(store_id, "BTC-CHAIN")]["recommendation"]
            self.assertEqual((recommendation["action"], recommendation["wallet"]), ("existing_wallet", "Shop Trezor"))
        # No Lightning node is connected, so BTCPay's payments are the balance.
        self.assertEqual(methods[("S1", "BTC-LN")]["recommendation"]["action"], "payment_ledger")
        self.assertEqual(methods[("S1", "XMR-CHAIN")]["recommendation"]["reason"], "non_bitcoin_asset")
        self.assertFalse(methods[("S1", "BTC-CHAIN")]["actions"]["wallet_source"]["available"])
        self.assertEqual(plan["sibling_backends"], ["cafe-history"])
        self.assertEqual(btcpay_setup.detected_network(plan), ("bitcoin", "regtest"))

    def test_history_key_imports_one_shared_wallet_once(self):
        self.conn.execute("DELETE FROM wallets WHERE id = 'trezor'")
        self.conn.commit()
        plan = self._plan(["btcpay.store.canmodifystoresettings"])
        methods = {(m["store_id"], m["payment_method_id"]): m for m in plan["payment_methods"]}

        self.assertEqual(methods[("S1", "BTC-CHAIN")]["recommendation"]["action"], "wallet_source")
        self.assertEqual(methods[("S2", "BTC-CHAIN")]["recommendation"]["reason"], "shared_wallet")

    def test_setup_rejects_importing_a_store_wallet_twice(self):
        plan = self._plan(["btcpay.store.canmodifystoresettings"])
        routes = btcpay_setup.routes_from_plan(
            plan,
            overrides={
                ("S1", "BTC-CHAIN"): {"action": "wallet_source"},
                ("S2", "BTC-CHAIN"): {"action": "wallet_source"},
            },
            use_recommendations=False,
        )
        with self.assertRaises(AppError) as ctx:
            btcpay_setup.validate_setup_routes(
                self.conn,
                "prof",
                routes,
                backend_name="shop-ro",
                runtime_config=self.runtime_config,
                server_url="https://pay.example.com",
            )
        self.assertEqual(ctx.exception.code, "conflict")

    def test_apply_creates_scoped_sources_mappings_and_provenance_routes(self):
        plan = self._plan(["btcpay.store.canmodifystoresettings"])
        routes = btcpay_setup.normalize_setup_routes(
            btcpay_setup.routes_from_plan(
                plan,
                overrides={("S2", "BTC-CHAIN"): {"action": "wallet_source"}},
            )
        )
        workspace = self.conn.execute("SELECT * FROM workspaces WHERE id = 'ws'").fetchone()
        profile = self.conn.execute("SELECT * FROM profiles WHERE id = 'prof'").fetchone()
        applied = btcpay_setup.apply_setup_routes(
            self.conn, workspace, profile, backend_name="shop-ro", routes=routes, label="Shop"
        )

        self.assertEqual(len(applied["wallet_sources"]), 1)
        source_config = applied["wallet_sources"][0]["config"]
        self.assertEqual(
            {key: source_config[key] for key in ("chain", "network", "store_id", "sync_source")},
            {"chain": "bitcoin", "network": "regtest", "store_id": "S2", "sync_source": "btcpay"},
        )
        self.assertTrue(source_config["btcpay_wallet_fingerprint"].startswith("btcpay-wallet:"))
        self.assertEqual(applied["mappings"][0]["wallet"]["label"], "Shop Trezor")
        self.assertEqual(applied["account_routes"], [])
        ledger = applied["payment_ledgers"][0]
        self.assertEqual(ledger["label"], "Shop - Main Store - Lightning")
        self.assertEqual(
            {key: ledger["config"][key] for key in ("source_mode", "payment_method_ids", "network")},
            {"source_mode": "payments", "payment_method_ids": ["BTC-LN"], "network": "regtest"},
        )
        self.assertEqual(applied["provenance_store_ids"], ["S1", "S2"])
        # Re-planning now sees the configured routes instead of new suggestions.
        replanned = {(m["store_id"], m["payment_method_id"]): m for m in self._plan(["btcpay.store.canmodifystoresettings"])["payment_methods"]}
        self.assertEqual(replanned[("S2", "BTC-CHAIN")]["recommendation"]["reason"], "already_configured")
        with self.assertRaises(AppError):
            btcpay_setup.validate_setup_routes(
                self.conn,
                "prof",
                [{"store_id": "S9", "payment_method_id": "BTC-CHAIN", "action": "wallet_source", "wallet_fingerprint": source_config["btcpay_wallet_fingerprint"]}],
                backend_name="shop-ro",
                runtime_config=self.runtime_config,
                server_url="https://pay.example.com",
            )

    def test_mapping_onto_a_btcpay_wallet_source_is_refused(self):
        workspace = self.conn.execute("SELECT * FROM workspaces WHERE id = 'ws'").fetchone()
        profile = self.conn.execute("SELECT * FROM profiles WHERE id = 'prof'").fetchone()
        applied = btcpay_setup.apply_setup_routes(
            self.conn,
            workspace,
            profile,
            backend_name="shop-ro",
            routes=btcpay_setup.normalize_setup_routes([{"store_id": "S1", "action": "wallet_source", "label": "Main source"}]),
            label="Shop",
        )
        self.assertEqual(applied["wallet_sources"][0]["label"], "Main source")
        with self.assertRaises(AppError):
            btcpay_setup.apply_setup_routes(
                self.conn,
                workspace,
                profile,
                backend_name="shop-ro",
                routes=btcpay_setup.normalize_setup_routes(
                    [{"store_id": "S2", "action": "existing_wallet", "wallet": "Main source"}]
                ),
                label="Shop",
            )


class PayoutProvenanceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-btcpay-payouts-")
        self.conn = open_db(str(Path(self.tmp.name) / "data"))
        self.conn.row_factory = sqlite3.Row
        now = now_iso()
        self.conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Shop', ?)", (now,))
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at) VALUES('prof', 'ws', 'Main', 'EUR', ?)",
            (now,),
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, created_at) VALUES('w', 'ws', 'prof', 'Store', 'custom', ?)",
            (now,),
        )
        self.txid = "ab" * 32
        for tx_id, direction in (("tx-out", "outbound"), ("tx-in", "inbound")):
            self.conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                    occurred_at, direction, asset, amount, fee, fiat_currency, kind, raw_json, created_at
                ) VALUES(?, 'ws', 'prof', 'w', ?, ?, '2026-01-02T00:00:00Z', ?, 'BTC', 10000000, 0, 'EUR', 'withdrawal', '{}', ?)
                """,
                (tx_id, self.txid, f"fp-{tx_id}", direction, now),
            )
        self.conn.commit()
        workspace = self.conn.execute("SELECT * FROM workspaces").fetchone()
        profile = self.conn.execute("SELECT * FROM profiles").fetchone()
        payout = normalize_payout(
            "S1",
            {
                "id": "po-1",
                "pullPaymentId": "pp-1",
                "payoutMethodId": "BTC-CHAIN",
                "originalAmount": "5.00",
                "originalCurrency": "EUR",
                "payoutCurrency": "BTC",
                "payoutAmount": "0.0001",
                "state": "Completed",
                "date": 1767312000,
                "paymentProof": {"proofType": "PayoutTransactionOnChainBlob", "transactionId": self.txid},
            },
            pull_payments={"pp-1": {"id": "pp-1", "name": "Refund 7yRWXHuDJaEyDXnrg8WJab"}},
        )
        commercial.upsert_btcpay_payouts(self.conn, workspace, profile, backend_name="shop", payouts=[payout])

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def _hooks(self):
        def resolve_scope(conn, workspace_ref=None, profile_ref=None):
            return conn.execute("SELECT * FROM workspaces").fetchone(), conn.execute("SELECT * FROM profiles").fetchone()

        return commercial.CommercialHooks(
            resolve_scope=resolve_scope,
            resolve_transaction=lambda conn, profile_id, ref: conn.execute(
                "SELECT * FROM transactions WHERE id = ?", (ref,)
            ).fetchone(),
            invalidate_journals=lambda conn, profile_id: None,
        )

    def test_refund_payout_is_suggested_for_the_outbound_leg_only(self):
        records = commercial.list_btcpay_records(self.conn, None, None, self._hooks(), record_type="payout")
        self.assertEqual(records[0]["origin_kind"], "refund")
        self.assertEqual(records[0]["invoice_id"], "7yRWXHuDJaEyDXnrg8WJab")
        suggested = commercial.suggest_links(self.conn, None, None, self._hooks())
        payout_links = [link for link in suggested["suggestions"] if link["transaction_id"] in ("tx-out", "tx-in")]
        self.assertEqual([link["transaction_id"] for link in payout_links], ["tx-out"])

        with self.assertRaises(AppError):
            commercial.review_link(
                self.conn, None, None, payout_links[0]["id"], self._hooks(), state="reviewed", commercial_kind="income"
            )
        reviewed = commercial.review_link(
            self.conn, None, None, payout_links[0]["id"], self._hooks(), state="reviewed", commercial_kind="refund"
        )
        self.assertTrue(reviewed["applied_to_transaction"])
        tx = self.conn.execute("SELECT kind, pricing_source_kind, fiat_value_exact FROM transactions WHERE id = 'tx-out'").fetchone()
        self.assertEqual((tx["kind"], tx["pricing_source_kind"]), ("refund", "btcpay_payout"))
        self.assertEqual(tx["fiat_value_exact"], "5.00")


if __name__ == "__main__":
    unittest.main()
