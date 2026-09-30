#!/usr/bin/env python3
"""Multi-store, multi-key BTCPay regtest scenario for the Greenfield connection.

Builds a merchant world that exercises what single-store seeding cannot:

- "Kassiber Regtest Store" (seeded by ``btcpay_seed``) and "Kassiber Pop-up
  Stand" pay into the same watch-only wallet, which Kassiber tracks as a
  descriptor wallet ("Shop Trezor").
- "Kassiber Café" has its own hot wallet, a Lightning method backed by an
  external node string, and LNURL. A Café sale is refunded and the refund
  payout is paid on-chain from the Café wallet.
- A real Point of Sale app and a payment request create invoices through
  BTCPay's own public endpoints, not simulated metadata.
- Two API keys: a read-only key for all stores, and a wallet-history key
  scoped to the Café only.

The Kassiber half runs the same CLI an operator would: ``btcpay inspect``,
``btcpay setup``, ``wallets sync``, and a refund review. Every assertion
failure raises, so the harness lane fails loudly.
"""

from __future__ import annotations

import argparse
import json
import os
import time
from decimal import Decimal
from pathlib import Path
from typing import Any
from urllib import parse, request

from dev.regtest.btcpay_seed import (
    DEFAULT_PASSWORD,
    DEFAULT_STORE_NAME,
    DEFAULT_USER,
    HttpFailure,
    _ensure_store,
    _fund_core_wallet_from_env,
    _json_request,
    _pay_regtest_invoice_from_core,
    _rpc_call,
    _run_kassiber,
    _run_kassiber_checked,
)

POPUP_STORE_NAME = "Kassiber Pop-up Stand"
CAFE_STORE_NAME = "Kassiber Café"
FAKE_LIGHTNING = "type=lnd-rest;server=https://127.0.0.1:1/;macaroon=0201036c6e640224030a10;allowinsecure=true"
PAYER_WALLET = "kassiber-btcpay-payer"


class ScenarioError(RuntimeError):
    pass


def _check(condition: bool, message: str, detail: Any = None) -> None:
    if not condition:
        suffix = f": {json.dumps(detail, default=str)[:600]}" if detail is not None else ""
        raise ScenarioError(message + suffix)


def _admin(base_url: str, method: str, path: str, body: dict[str, Any] | None = None) -> Any:
    return _json_request(base_url, method, path, body=body, basic=(DEFAULT_USER, DEFAULT_PASSWORD))


def _store_wallet_config(base_url: str, store_id: str) -> dict[str, Any]:
    payload = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN?includeConfig=true")
    config = payload.get("config") if isinstance(payload, dict) else None
    _check(isinstance(config, dict) and config.get("accountDerivation"), "main store wallet has no derivation", payload)
    return config


def _ensure_shared_wallet_store(base_url: str, derivation: str) -> str:
    store = _ensure_store(base_url, DEFAULT_USER, DEFAULT_PASSWORD, POPUP_STORE_NAME)
    store_id = str(store["id"])
    try:
        current = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN?includeConfig=true")
    except HttpFailure as exc:
        if exc.status != 404:
            raise
        current = None
    if not (isinstance(current, dict) and (current.get("config") or {}).get("accountDerivation") == derivation):
        _admin(
            base_url,
            "PUT",
            f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN",
            {"enabled": True, "config": {"derivationScheme": derivation, "label": "Shared with the main store"}},
        )
    return store_id


def _ensure_cafe_store(base_url: str) -> str:
    store = _ensure_store(base_url, DEFAULT_USER, DEFAULT_PASSWORD, CAFE_STORE_NAME)
    store_id = str(store["id"])
    try:
        _admin(
            base_url,
            "POST",
            f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN/wallet/generate",
            {"savePrivateKeys": True, "scriptPubKeyType": "Segwit", "wordCount": 12},
        )
    except HttpFailure as exc:
        if exc.status != 400:
            raise
    _admin(base_url, "PUT", f"/api/v1/stores/{store_id}/payment-methods/BTC-LN", {"enabled": True, "config": FAKE_LIGHTNING})
    _admin(
        base_url,
        "PUT",
        f"/api/v1/stores/{store_id}/payment-methods/BTC-LNURL",
        {"enabled": True, "config": {"useBech32Scheme": True, "lud12Enabled": True}},
    )
    return store_id


def _onchain_destination(base_url: str, store_id: str, invoice_id: str) -> tuple[str, str]:
    methods = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/invoices/{invoice_id}/payment-methods")
    method = next(item for item in methods if item.get("paymentMethodId") == "BTC-CHAIN")
    return str(method["destination"]), str(method["due"])


def _wait_invoice(base_url: str, store_id: str, invoice_id: str, *, seconds: int = 90) -> dict[str, Any]:
    deadline = time.monotonic() + seconds
    while True:
        invoice = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/invoices/{invoice_id}")
        if invoice.get("status") == "Settled":
            return invoice
        _check(time.monotonic() < deadline, f"invoice {invoice_id} did not settle", invoice.get("status"))
        time.sleep(2)


def _pay_invoice(base_url: str, store_id: str, invoice_id: str) -> str:
    destination, due = _onchain_destination(base_url, store_id, invoice_id)
    txid = _pay_regtest_invoice_from_core(destination, due, PAYER_WALLET)
    _wait_invoice(base_url, store_id, invoice_id)
    return txid


def _find_invoice(base_url: str, store_id: str, order_id: str) -> dict[str, Any] | None:
    query = parse.urlencode({"orderId": order_id})
    invoices = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/invoices?{query}")
    return invoices[0] if invoices else None


def _pos_sale(base_url: str, store_id: str) -> str:
    """Sell an espresso through the real PoS app route."""

    apps = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/apps")
    app = next((item for item in apps if item.get("appName") == "Pop-up PoS"), None)
    if app is None:
        app = _admin(
            base_url,
            "POST",
            f"/api/v1/stores/{store_id}/apps/pos",
            {
                "appName": "Pop-up PoS",
                "title": "Pop-up PoS",
                "currency": "BTC",
                "template": json.dumps([{"id": "espresso", "title": "Espresso", "price": "0.00002", "priceType": "Fixed"}]),
            },
        )
    existing = [
        invoice
        for invoice in _admin(base_url, "GET", f"/api/v1/stores/{store_id}/invoices?take=50")
        if f"/apps/{app['id']}/pos" in str((invoice.get("metadata") or {}).get("orderUrl") or "")
        and invoice.get("status") == "Settled"
    ]
    if existing:
        return str(existing[0]["id"])

    class _NoRedirect(request.HTTPRedirectHandler):
        def redirect_request(self, *_args, **_kwargs):
            return None

    opener = request.build_opener(_NoRedirect)
    req = request.Request(
        f"{base_url.rstrip('/')}/apps/{app['id']}/pos",
        data=parse.urlencode({"choiceKey": "espresso"}).encode(),
        method="POST",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        opener.open(req, timeout=30)
        raise ScenarioError("PoS checkout did not redirect to an invoice")
    except request.HTTPError as exc:
        location = exc.headers.get("Location") or ""
    invoice_id = location.rstrip("/").rsplit("/", 1)[-1].split("=")[-1]
    _pay_invoice(base_url, store_id, invoice_id)
    return invoice_id


def _payment_request_sale(base_url: str, store_id: str) -> str:
    requests_ = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/payment-requests")
    payment_request = next((item for item in requests_ if item.get("title") == "Catering deposit"), None)
    if payment_request is None:
        payment_request = _admin(
            base_url,
            "POST",
            f"/api/v1/stores/{store_id}/payment-requests",
            {"title": "Catering deposit", "amount": "0.0003", "currency": "BTC"},
        )
    existing = _find_invoice(base_url, store_id, f"PAY_REQUEST_{payment_request['id']}")
    if existing and existing.get("status") == "Settled":
        return str(existing["id"])

    class _NoRedirect(request.HTTPRedirectHandler):
        def redirect_request(self, *_args, **_kwargs):
            return None

    opener = request.build_opener(_NoRedirect)
    try:
        opener.open(f"{base_url.rstrip('/')}/payment-requests/{payment_request['id']}/pay", timeout=30)
        raise ScenarioError("payment request did not redirect to an invoice")
    except request.HTTPError as exc:
        location = exc.headers.get("Location") or ""
    invoice_id = location.rstrip("/").rsplit("/", 1)[-1].split("=")[-1]
    _pay_invoice(base_url, store_id, invoice_id)
    return invoice_id


def _refunded_sale(base_url: str, store_id: str) -> dict[str, Any]:
    """A Café sale refunded in EUR and paid out on-chain from the Café wallet."""

    invoice = _find_invoice(base_url, store_id, "cafe-lunch-refund")
    if invoice is None:
        invoice = _admin(
            base_url,
            "POST",
            f"/api/v1/stores/{store_id}/invoices",
            {
                "amount": "0.0005",
                "currency": "BTC",
                "metadata": {"orderId": "cafe-lunch-refund", "itemDesc": "Lunch for two"},
                "checkout": {"paymentMethods": ["BTC-CHAIN"]},
            },
        )
    if invoice.get("status") != "Settled":
        _pay_invoice(base_url, store_id, str(invoice["id"]))
    payouts = _admin(base_url, "GET", f"/api/v1/stores/{store_id}/payouts")
    pulls = {item["id"]: item for item in _admin(base_url, "GET", f"/api/v1/stores/{store_id}/pull-payments")}
    refund_name = f"Refund {invoice['id']}"
    payout = next(
        (item for item in payouts if pulls.get(item.get("pullPaymentId"), {}).get("name") == refund_name),
        None,
    )
    if payout is None:
        pull = _admin(
            base_url,
            "POST",
            f"/api/v1/stores/{store_id}/invoices/{invoice['id']}/refund",
            {"refundVariant": "Custom", "paymentMethod": "BTC-CHAIN", "customAmount": "10", "customCurrency": "EUR"},
        )
        core = _fund_core_wallet_from_env(PAYER_WALLET, required_btc=Decimal("0.01"))
        _check(core is not None, "regtest Core credentials are required")
        url, user, password = core
        destination = _rpc_call(f"{url.rstrip('/')}/wallet/{PAYER_WALLET}", user, password, "getnewaddress")
        payout = _json_request(
            base_url,
            "POST",
            f"/api/v1/pull-payments/{pull['id']}/payouts",
            body={"destination": destination, "payoutMethodId": "BTC-CHAIN"},
        )
        payout = _admin(base_url, "POST", f"/api/v1/stores/{store_id}/payouts/{payout['id']}", {"revision": payout.get("revision", 0)})
    if payout.get("state") != "Completed":
        if payout.get("state") == "AwaitingPayment":
            _admin(
                base_url,
                "POST",
                f"/api/v1/stores/{store_id}/payment-methods/BTC-CHAIN/wallet/transactions",
                {
                    "destinations": [{"destination": payout["destination"], "amount": payout["payoutAmount"]}],
                    "feerate": 2,
                    "proceedWithBroadcast": True,
                },
            )
        core = _fund_core_wallet_from_env(PAYER_WALLET, required_btc=Decimal("0"))
        url, user, password = core
        miner = _rpc_call(f"{url.rstrip('/')}/wallet/{PAYER_WALLET}", user, password, "getnewaddress")
        _rpc_call(url, user, password, "generatetoaddress", [1, miner])
        deadline = time.monotonic() + 90
        while payout.get("state") != "Completed":
            _check(time.monotonic() < deadline, "refund payout was not marked completed", payout)
            time.sleep(2)
            payout = next(item for item in _admin(base_url, "GET", f"/api/v1/stores/{store_id}/payouts") if item["id"] == payout["id"])
    return {"invoice_id": str(invoice["id"]), "payout_id": str(payout["id"]), "proof": payout.get("paymentProof")}


def _create_key(base_url: str, label: str, permissions: list[str]) -> str:
    payload = _admin(base_url, "POST", "/api/v1/api-keys", {"label": label, "permissions": permissions})
    return str(payload["apiKey"])


def _methods_by_key(plan: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
    return {(item["store_id"], item["payment_method_id"]): item for item in plan.get("payment_methods") or []}


def exercise_multi_store(*, base_url: str, data_root: Path, electrum_url: str) -> dict[str, Any]:
    stores = _admin(base_url, "GET", "/api/v1/stores")
    main_store = next(store for store in stores if store.get("name") == DEFAULT_STORE_NAME)
    main_id = str(main_store["id"])
    main_wallet = _store_wallet_config(base_url, main_id)
    popup_id = _ensure_shared_wallet_store(base_url, str(main_wallet["accountDerivation"]))
    cafe_id = _ensure_cafe_store(base_url)
    pos_invoice = _pos_sale(base_url, popup_id)
    request_invoice = _payment_request_sale(base_url, cafe_id)
    refund = _refunded_sale(base_url, cafe_id)
    read_only_key = _create_key(base_url, "Kassiber read-only", ["btcpay.store.canviewstoresettings"])
    history_key = _create_key(base_url, "Kassiber Café history", [f"btcpay.store.canmodifystoresettings:{cafe_id}"])

    common = ["--data-root", str(data_root), "--machine"]
    for args in (
        ["init"],
        ["workspaces", "create", "Merchant"],
        ["profiles", "create", "--workspace", "Merchant", "--fiat-currency", "EUR", "Shops"],
        ["context", "set", "--workspace", "Merchant", "--profile", "Shops"],
    ):
        _run_kassiber([*common, *args])
    _run_kassiber_checked(
        [*common, "backends", "create", "regtest-electrum", "--kind", "electrum", "--url", electrum_url, "--chain", "bitcoin", "--network", "regtest"]
    )
    # A pasted browser URL: the client normalises /api/v1 and trailing slashes.
    _run_kassiber_checked(
        [*common, "backends", "create", "shop-readonly", "--kind", "btcpay", "--url", f"{base_url.rstrip('/')}/api/v1/", "--network", "regtest", "--token-stdin"],
        token=read_only_key,
    )
    _run_kassiber_checked(
        [*common, "backends", "create", "cafe-history", "--kind", "btcpay", "--url", base_url, "--network", "regtest", "--token-stdin"],
        token=history_key,
    )
    key = main_wallet["accountKeySettings"][0]
    origin = f"[{key['rootFingerprint']}/{key['accountKeyPath'].replace(chr(39), 'h')}]{key['accountKey']}"
    _run_kassiber_checked(
        [
            *common, "wallets", "create", "--label", "Shop Trezor", "--kind", "descriptor",
            "--backend", "regtest-electrum", "--chain", "bitcoin", "--network", "regtest",
            "--descriptor", f"wpkh({origin}/0/*)", "--change-descriptor", f"wpkh({origin}/1/*)",
        ]
    )

    key_url = _run_kassiber_checked([*common, "btcpay", "key-url", "--backend", "shop-readonly"])
    _check("/api-keys/authorize?" in str(key_url.get("authorize_url")), "key-url did not build an authorize link", key_url)

    plan = _run_kassiber_checked([*common, "btcpay", "inspect", "--backend", "shop-readonly"])
    methods = _methods_by_key(plan)
    _check(plan["api_key"]["risk"] == "read_only", "read-only key was not classified read-only", plan["api_key"])
    _check(plan["detected_network"] == "regtest", "regtest network was not detected", plan.get("detected_network"))
    for store_id in (main_id, popup_id):
        recommendation = methods[(store_id, "BTC-CHAIN")]["recommendation"]
        _check(
            recommendation["action"] == "existing_wallet" and recommendation["wallet"] == "Shop Trezor",
            "shared store wallet was not recognised as the tracked descriptor wallet",
            recommendation,
        )
    # No Lightning node is connected to the book, so BTCPay's payments are the balance.
    _check(
        methods[(cafe_id, "BTC-LN")]["recommendation"]["action"] == "payment_ledger",
        "Lightning was not booked as a BTCPay payment ledger",
        methods[(cafe_id, "BTC-LN")]["recommendation"],
    )
    _check(methods[(cafe_id, "BTC-LNURL")]["rail"] == "lnurl", "LNURL rail was not classified")
    _check(
        methods[(cafe_id, "BTC-CHAIN")]["recommendation"]["reason"] == "read_only_key_upgrade",
        "read-only Café wallet did not explain the wallet-history key",
        methods[(cafe_id, "BTC-CHAIN")]["recommendation"],
    )
    _check("shared_store_wallet" in [w["code"] for w in plan["warnings"]], "shared wallet warning missing", plan["warnings"])
    _check(read_only_key not in json.dumps(plan), "API key leaked into the plan")
    _check("tpub" not in json.dumps(plan), "wallet key material leaked into the plan")

    applied = _run_kassiber_checked(
        [*common, "btcpay", "setup", "--backend", "shop-readonly", "--label", "Shops", "--recommended"]
    )
    _check(len(applied["mappings"]) == 2, "both shared stores should map to Shop Trezor", applied["mappings"])
    _check(not applied["wallet_sources"], "a read-only key must not create BTCPay wallet sources", applied["wallet_sources"])
    ledgers = applied.get("payment_ledgers") or []
    _check(
        len(ledgers) == 1 and ledgers[0]["config"].get("payment_method_ids") == ["BTC-LN", "BTC-LNURL"],
        "Lightning and LNURL should share one Café payment ledger",
        ledgers,
    )
    history_plan = _run_kassiber_checked([*common, "btcpay", "inspect", "--backend", "cafe-history"])
    history_methods = _methods_by_key(history_plan)
    _check(history_plan["api_key"]["risk"] == "can_modify_store", "history key risk not flagged", history_plan["api_key"])
    _check(history_plan["sibling_backends"] == ["shop-readonly"], "sibling key for the same server not reported", history_plan["sibling_backends"])
    _check(
        history_methods[(cafe_id, "BTC-CHAIN")]["recommendation"]["action"] == "wallet_source",
        "history key should allow importing the Café wallet",
        history_methods[(cafe_id, "BTC-CHAIN")],
    )
    cafe_setup = _run_kassiber_checked(
        [*common, "btcpay", "setup", "--backend", "cafe-history", "--label", "Café", "--route", f"{cafe_id}:BTC-CHAIN=wallet_source"]
    )
    _check(len(cafe_setup["wallet_sources"]) == 1, "Café wallet source not created", cafe_setup)

    synced = _run_kassiber_checked([*common, "wallets", "sync", "--all"])
    statuses = {row["wallet"]: row["status"] for row in synced}
    _check(all(status == "synced" for status in statuses.values()), "a wallet failed to sync", synced)

    payouts = _run_kassiber_checked([*common, "btcpay", "provenance", "list", "--record-type", "payout"])
    refund_record = next((row for row in payouts if row["payment_id"] == refund["payout_id"]), None)
    _check(refund_record is not None and refund_record["origin_kind"] == "refund", "refund payout not captured", payouts)
    _check(refund_record["invoice_id"] == refund["invoice_id"] and refund_record["txid"], "refund payout lost its invoice or txid", refund_record)
    links = _run_kassiber_checked([*common, "btcpay", "provenance", "links", "--state", "suggested", "--limit", "500"])
    refund_link = next((link for link in links if link.get("payment_id") == refund["payout_id"]), None)
    _check(refund_link is not None, "refund payout was not suggested against the outbound transaction", links)
    reviewed = _run_kassiber_checked(
        [*common, "btcpay", "provenance", "review", "--link", refund_link["id"], "--state", "reviewed", "--commercial-kind", "refund"]
    )
    _check(reviewed["applied_to_transaction"], "refund review did not apply to the transaction", reviewed)
    # BTCPay reports the refund send net of its miner fee; the completed payout
    # explains the rest, so the fee is booked separately.
    refund_rows = _run_kassiber_checked(
        [*common, "transactions", "list", "--txid", refund_record["txid"], "--direction", "outbound"]
    )
    refund_rows = refund_rows.get("transactions", refund_rows) if isinstance(refund_rows, dict) else refund_rows
    refund_fee = max((int(row.get("fee_msat") or 0) for row in refund_rows), default=0)
    _check(0 < refund_fee < 1_000_000_000, "refund send did not book its miner fee separately", refund_rows)
    refreshed = _run_kassiber_checked([*common, "btcpay", "inspect", "--backend", "cafe-history"])
    cafe_state = next((store.get("sync_state") for store in refreshed["stores"] if store["id"] == cafe_id), None)
    _check(
        cafe_state is not None and cafe_state.get("last_success_at") and not cafe_state.get("stale"),
        "synced Café store should report fresh BTCPay data",
        cafe_state,
    )
    invoices = _run_kassiber_checked([*common, "btcpay", "provenance", "list", "--record-type", "invoice", "--limit", "500"])
    origins = {row["invoice_id"]: row for row in invoices}
    _check(origins.get(pos_invoice, {}).get("origin_kind") == "pos", "PoS app invoice origin not recognised", origins.get(pos_invoice))
    _check(
        origins.get(request_invoice, {}).get("origin_kind") == "payment_request"
        and origins[request_invoice]["origin_label"] == "Catering deposit",
        "payment request invoice not labelled from the payment request",
        origins.get(request_invoice),
    )
    store_payment_links = [link for link in links if link.get("payment_id") != refund["payout_id"]]
    return {
        "stores": {"main": main_id, "popup": popup_id, "cafe": cafe_id},
        "read_only_mappings": len(applied["mappings"]),
        "read_only_account_routes": len(applied["account_routes"]),
        "payment_ledgers": len(ledgers),
        "refund_fee_msat": refund_fee,
        "cafe_wallet_sources": len(cafe_setup["wallet_sources"]),
        "wallets_synced": sorted(statuses),
        "refund_payout": refund["payout_id"],
        "refund_review_applied": bool(reviewed["applied_to_transaction"]),
        "store_payment_suggestions": len(store_payment_links),
        "pos_invoice": pos_invoice,
        "payment_request_invoice": request_invoice,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--kassiber-data-root", required=True)
    parser.add_argument(
        "--electrum-url",
        default=f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT', '18543')}",
    )
    parser.add_argument("--json-output")
    args = parser.parse_args(argv)
    summary = exercise_multi_store(
        base_url=args.base_url,
        data_root=Path(args.kassiber_data_root),
        electrum_url=args.electrum_url,
    )
    text = json.dumps(summary, indent=2, sort_keys=True)
    if args.json_output:
        output = Path(args.json_output)
        output.write_text(text + "\n", encoding="utf-8")
        os.chmod(output, 0o600)
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
