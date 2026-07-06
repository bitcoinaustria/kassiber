#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import time
from decimal import Decimal
from pathlib import Path
from typing import Any
from urllib import error, parse, request


DEFAULT_USER = "regtest@regtest.local"
DEFAULT_PASSWORD = "regtest"
DEFAULT_STORE_NAME = "Kassiber Regtest Store"
DEFAULT_BACKEND_NAME = "btcpay-regtest"
DEFAULT_WALLET_LABEL = "BTCPay Regtest Store"
DEFAULT_PAYMENT_METHOD_ID = "BTC-CHAIN"
DEFAULT_WORKSPACE = "Regtest Demo"
DEFAULT_PROFILE = "Full Accounting"
DEFAULT_ORDER_ID = "kassiber-regtest-btcpay-smoke"
DEMO_COMMERCIAL_TAGS = (
    ("btcpay", "BTCPay"),
    ("payment-request", "Payment request"),
    ("commercial-income", "Commercial income"),
)


REGTEST_INVOICE_SCENARIOS = (
    {
        "kind": "direct_invoice",
        "suffix": "direct",
        "amount": "0.00021000",
        "metadata": {
            "itemDesc": "Direct Greenfield invoice",
            "buyerName": "Kassiber Regtest Buyer",
        },
    },
    {
        "kind": "duplicate_order_adjustment",
        "suffix": "direct",
        "amount": "0.00003000",
        "metadata": {
            "itemDesc": "Adjustment invoice for the original order",
            "buyerName": "Kassiber Regtest Buyer",
            "buyerEmail": "merchant-customer@example.invalid",
        },
    },
    {
        "kind": "pos",
        "suffix": "pos-coffee",
        "amount": "0.00012000",
        "metadata": {
            "itemDesc": "Point-of-sale coffee",
            "posData": {"title": "Coffee bag", "quantity": 1},
            "orderUrl": "/apps/pos/kassiber-regtest-pos",
        },
    },
    {
        "kind": "partial_payment",
        "suffix": "partial-payment",
        "amount": "0.00023000",
        "payment_plan": ("0.00007000", "remaining"),
        "metadata": {
            "itemDesc": "Invoice paid in two on-chain transactions",
            "buyerEmail": "split-payer@example.invalid",
            "orderUrl": "/orders/kassiber-regtest-partial-payment",
        },
    },
    {
        "kind": "fiat_eur_invoice",
        "suffix": "eur-checkout",
        "amount": "11.00",
        "currency": "EUR",
        "metadata": {
            "itemDesc": "EUR-denominated shop checkout",
            "buyerEmail": "eur-buyer@example.invalid",
            "orderUrl": "/orders/kassiber-regtest-eur-checkout",
        },
    },
    {
        "kind": "payment_request",
        "suffix": "payment-request",
        "amount": "15.00",
        "currency": "EUR",
        "metadata": {
            "itemDesc": "Monthly association membership",
            "paymentRequestId": "kassiber-regtest-membership-request",
            "orderUrl": "/payment-requests/kassiber-regtest-membership",
        },
    },
    {
        "kind": "crowdfund",
        "suffix": "crowdfund",
        "amount": "0.00018000",
        "metadata": {
            "itemDesc": "Crowdfund supporter pledge",
            "appId": "kassiber-regtest-crowdfund",
            "appName": "Kassiber Crowdfund",
            "orderUrl": "/apps/crowdfund/kassiber-regtest-campaign",
        },
    },
)


class HttpFailure(RuntimeError):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(f"HTTP {status}: {body[:300]}")
        self.status = status
        self.body = body


def _rpc_call(url: str, user: str, password: str, method: str, params: list[Any] | None = None) -> Any:
    payload = json.dumps(
        {"jsonrpc": "1.0", "id": f"kassiber-btcpay-{method}", "method": method, "params": params or []}
    ).encode("utf-8")
    req = request.Request(url.rstrip("/"), data=payload, headers={"Content-Type": "application/json"})
    raw = f"{user}:{password}".encode("utf-8")
    req.add_header("Authorization", "Basic " + base64.b64encode(raw).decode("ascii"))
    with request.urlopen(req, timeout=30) as response:
        body = json.loads(response.read().decode("utf-8"))
    if body.get("error"):
        raise RuntimeError(f"{method} failed: {body['error']}")
    return body.get("result")


def _warm_core_from_env() -> None:
    url = os.environ.get("KASSIBER_REGTEST_CORE_URL")
    user = os.environ.get("KASSIBER_REGTEST_RPC_USER")
    password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD")
    if not url or not user or not password:
        return
    wallet_name = os.environ.get("KASSIBER_REGTEST_BTCPAY_READY_WALLET", "kassiber-btcpay-ready")
    try:
        loaded = _rpc_call(url, user, password, "listwallets")
        if wallet_name not in loaded:
            try:
                _rpc_call(url, user, password, "createwallet", [wallet_name])
            except RuntimeError:
                _rpc_call(url, user, password, "loadwallet", [wallet_name])
        wallet_url = f"{url.rstrip('/')}/wallet/{wallet_name}"
        address = _rpc_call(wallet_url, user, password, "getnewaddress")
        _rpc_call(url, user, password, "generatetoaddress", [1, address])
    except Exception:
        # Best-effort only. BTCPay readiness is still proven by the API calls below.
        return


def _ensure_core_wallet(url: str, user: str, password: str, wallet_name: str) -> None:
    loaded = _rpc_call(url, user, password, "listwallets") or []
    if wallet_name in loaded:
        return
    try:
        _rpc_call(url, user, password, "createwallet", [wallet_name])
    except RuntimeError:
        _rpc_call(url, user, password, "loadwallet", [wallet_name])


def _fund_core_wallet_from_env(wallet_name: str, *, required_btc: Decimal) -> tuple[str, str, str] | None:
    url = os.environ.get("KASSIBER_REGTEST_CORE_URL")
    user = os.environ.get("KASSIBER_REGTEST_RPC_USER")
    password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD")
    if not url or not user or not password:
        return None
    _ensure_core_wallet(url, user, password, wallet_name)
    wallet_url = f"{url.rstrip('/')}/wallet/{wallet_name}"
    balance = Decimal(str(_rpc_call(wallet_url, user, password, "getbalance") or "0"))
    if balance < required_btc:
        mining_address = _rpc_call(wallet_url, user, password, "getnewaddress")
        _rpc_call(url, user, password, "generatetoaddress", [101, mining_address])
    return url, user, password


def _pay_regtest_invoice_from_core(destination: str, amount_btc: str, wallet_name: str) -> str:
    core = _fund_core_wallet_from_env(wallet_name, required_btc=Decimal(amount_btc) + Decimal("0.001"))
    if core is None:
        raise RuntimeError("KASSIBER_REGTEST_CORE_URL/RPC credentials are required to pay the BTCPay invoice")
    url, user, password = core
    wallet_url = f"{url.rstrip('/')}/wallet/{wallet_name}"
    txid = _rpc_call(wallet_url, user, password, "sendtoaddress", [destination, amount_btc])
    mining_address = _rpc_call(wallet_url, user, password, "getnewaddress")
    _rpc_call(url, user, password, "generatetoaddress", [1, mining_address])
    return str(txid)


def _json_request(
    base_url: str,
    method: str,
    path: str,
    *,
    body: dict[str, Any] | None = None,
    token: str | None = None,
    basic: tuple[str, str] | None = None,
    timeout: int = 30,
) -> Any:
    data = None if body is None else json.dumps(body).encode("utf-8")
    headers = {"Accept": "application/json"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"token {token}"
    if basic:
        raw = f"{basic[0]}:{basic[1]}".encode("utf-8")
        headers["Authorization"] = "Basic " + base64.b64encode(raw).decode("ascii")
    req = request.Request(
        base_url.rstrip("/") + path,
        data=data,
        headers=headers,
        method=method,
    )
    try:
        with request.urlopen(req, timeout=timeout) as response:
            payload = response.read().decode("utf-8")
    except error.HTTPError as exc:
        payload = exc.read().decode("utf-8", errors="replace")
        raise HttpFailure(exc.code, payload) from exc
    if not payload.strip():
        return None
    return json.loads(payload)


def _wait_for_btcpay(base_url: str, *, deadline_seconds: int) -> None:
    deadline = time.monotonic() + deadline_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            _json_request(
                base_url,
                "GET",
                "/api/v1/stores",
                basic=(DEFAULT_USER, DEFAULT_PASSWORD),
                timeout=5,
            )
            return
        except HttpFailure as exc:
            last_error = exc
            if exc.status in {401, 403, 404}:
                return
        except Exception as exc:
            last_error = exc
        time.sleep(2)
    raise RuntimeError(f"BTCPay did not become reachable at {base_url}: {last_error}")


def _ensure_user(base_url: str, user: str, password: str) -> None:
    try:
        _json_request(
            base_url,
            "POST",
            "/api/v1/users",
            body={"email": user, "password": password, "isAdministrator": True},
        )
    except HttpFailure as exc:
        if exc.status == 401:
            _json_request(base_url, "GET", "/api/v1/users/me", basic=(user, password))
            return
        if exc.status in {400, 409, 422}:
            return
        raise


def _ensure_store(base_url: str, user: str, password: str, store_name: str) -> dict[str, Any]:
    stores = _json_request(base_url, "GET", "/api/v1/stores", basic=(user, password))
    if isinstance(stores, list):
        for store in stores:
            if isinstance(store, dict) and store.get("name") == store_name:
                return store
    created = _json_request(
        base_url,
        "POST",
        "/api/v1/stores",
        body={"name": store_name, "defaultCurrency": "EUR"},
        basic=(user, password),
    )
    if not isinstance(created, dict) or not created.get("id"):
        raise RuntimeError(f"Unexpected BTCPay store response: {created!r}")
    return created


def _payment_method_configured(
    base_url: str,
    user: str,
    password: str,
    store_id: str,
    payment_method_id: str,
) -> bool:
    try:
        payload = _json_request(
            base_url,
            "GET",
            f"/api/v1/stores/{store_id}/payment-methods/{payment_method_id}?includeConfig=true",
            basic=(user, password),
        )
    except HttpFailure as exc:
        if exc.status == 404:
            return False
        raise
    if not isinstance(payload, dict):
        return False
    config = payload.get("config")
    return isinstance(config, dict) and bool(config.get("derivationScheme"))


def _ensure_wallet(
    base_url: str,
    user: str,
    password: str,
    store_id: str,
    payment_method_id: str,
) -> bool:
    if _payment_method_configured(base_url, user, password, store_id, payment_method_id):
        return False
    body = {
        "label": DEFAULT_WALLET_LABEL,
        "accountNumber": 0,
        "savePrivateKeys": False,
        "wordList": "English",
        "wordCount": 12,
        "scriptPubKeyType": "Segwit",
    }
    deadline = time.monotonic() + 180
    while True:
        try:
            _json_request(
                base_url,
                "POST",
                f"/api/v1/stores/{store_id}/payment-methods/{payment_method_id}/wallet/generate",
                body=body,
                basic=(user, password),
            )
            break
        except HttpFailure as exc:
            try:
                error_payload = json.loads(exc.body)
            except json.JSONDecodeError:
                error_payload = None
            if (
                exc.status == 400
                and isinstance(error_payload, dict)
                and error_payload.get("code") == "already-configured"
            ):
                return False
            if exc.status != 503 or time.monotonic() >= deadline:
                raise
            time.sleep(3)
    return True


def _ensure_lightning(
    base_url: str,
    user: str,
    password: str,
    store_id: str,
    connection_string: str | None,
) -> bool:
    if not connection_string:
        return False
    _json_request(
        base_url,
        "PUT",
        f"/api/v1/stores/{store_id}/payment-methods/BTC-LN",
        body={"enabled": True, "config": connection_string},
        basic=(user, password),
    )
    return True


def _create_api_key(base_url: str, user: str, password: str, store_id: str) -> str:
    permissions = [
        f"btcpay.store.canmodifystoresettings:{store_id}",
        f"btcpay.store.canviewinvoices:{store_id}",
        f"btcpay.store.cancreateinvoice:{store_id}",
        f"btcpay.store.canmodifyinvoices:{store_id}",
    ]
    payload = _json_request(
        base_url,
        "POST",
        "/api/v1/api-keys",
        body={"label": "Kassiber regtest", "permissions": permissions},
        basic=(user, password),
    )
    if not isinstance(payload, dict) or not payload.get("apiKey"):
        raise RuntimeError(f"Unexpected BTCPay API key response: {payload!r}")
    return str(payload["apiKey"])


def _find_invoice_by_reference(
    base_url: str,
    token: str,
    store_id: str,
    *,
    order_id: str | None = None,
    metadata_key: str | None = None,
    metadata_value: str | None = None,
) -> dict[str, Any] | None:
    params = {
        "includePaymentMethods": "true",
        "take": "50",
    }
    if order_id:
        params["orderId"] = order_id
    payload = _json_request(
        base_url,
        "GET",
        f"/api/v1/stores/{store_id}/invoices?{parse.urlencode(params)}",
        token=token,
    )
    invoices = payload.get("data") if isinstance(payload, dict) else payload
    if not isinstance(invoices, list):
        return None
    for invoice in invoices:
        if not isinstance(invoice, dict):
            continue
        metadata = invoice.get("metadata")
        if (
            metadata_key
            and metadata_value
            and isinstance(metadata, dict)
            and str(metadata.get(metadata_key) or "") == metadata_value
        ):
            return invoice
        if metadata_key and metadata_value:
            continue
        if order_id and isinstance(metadata, dict) and metadata.get("orderId") == order_id:
            return invoice
        if order_id and invoice.get("orderId") == order_id:
            return invoice
    return None


def _create_or_get_invoice(
    base_url: str,
    token: str,
    store_id: str,
    *,
    order_id: str | None,
    amount: str,
    currency: str,
    metadata: dict[str, Any],
    metadata_key: str | None = None,
    metadata_value: str | None = None,
) -> dict[str, Any]:
    existing = _find_invoice_by_reference(
        base_url,
        token,
        store_id,
        order_id=order_id,
        metadata_key=metadata_key,
        metadata_value=metadata_value,
    )
    if existing is not None:
        return existing
    invoice_metadata = dict(metadata)
    if order_id:
        invoice_metadata.setdefault("orderId", order_id)
    payload = _json_request(
        base_url,
        "POST",
        f"/api/v1/stores/{store_id}/invoices",
        body={
            "amount": amount,
            "currency": currency,
            "metadata": invoice_metadata,
        },
        token=token,
    )
    if not isinstance(payload, dict) or not payload.get("id"):
        raise RuntimeError(f"Unexpected BTCPay invoice response: {payload!r}")
    return payload


def _invoice_payment_methods(base_url: str, token: str, store_id: str, invoice_id: str) -> list[dict[str, Any]]:
    payload = _json_request(
        base_url,
        "GET",
        f"/api/v1/stores/{store_id}/invoices/{invoice_id}/payment-methods",
        token=token,
    )
    if not isinstance(payload, list):
        raise RuntimeError(f"Unexpected BTCPay invoice payment methods response: {payload!r}")
    return [method for method in payload if isinstance(method, dict)]


def _payment_method_for_invoice(
    base_url: str,
    token: str,
    store_id: str,
    invoice_id: str,
    payment_method_id: str,
) -> dict[str, Any]:
    methods = _invoice_payment_methods(base_url, token, store_id, invoice_id)
    for method in methods:
        if str(method.get("paymentMethodId") or "") == payment_method_id:
            return method
    raise RuntimeError(f"Invoice {invoice_id} did not expose payment method {payment_method_id}")


def _wait_for_invoice_settlement(
    base_url: str,
    token: str,
    store_id: str,
    invoice_id: str,
    *,
    deadline_seconds: int,
) -> dict[str, Any]:
    deadline = time.monotonic() + deadline_seconds
    last_invoice: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        payload = _json_request(
            base_url,
            "GET",
            f"/api/v1/stores/{store_id}/invoices/{invoice_id}",
            token=token,
        )
        if isinstance(payload, dict):
            last_invoice = payload
            if str(payload.get("status") or "") == "Settled":
                return payload
        time.sleep(2)
    raise RuntimeError(f"BTCPay invoice {invoice_id} did not settle in time: {last_invoice!r}")


def _invoice_is_settled(invoice: dict[str, Any]) -> bool:
    return str(invoice.get("status") or "").lower() == "settled"


def _format_btc_amount(value: Decimal) -> str:
    return f"{value.quantize(Decimal('0.00000001')):.8f}"


def _method_due_btc(method: dict[str, Any]) -> Decimal:
    return Decimal(str(method.get("due") or method.get("amount") or "0"))


def _method_payment_count(method: dict[str, Any]) -> int:
    payments = method.get("payments")
    return len(payments) if isinstance(payments, list) else 0


def _wait_for_payment_method_update(
    *,
    base_url: str,
    api_key: str,
    store_id: str,
    invoice_id: str,
    payment_method_id: str,
    previous_due: Decimal,
    expected_payment_count: int,
    deadline_seconds: int,
) -> dict[str, Any]:
    deadline = time.monotonic() + deadline_seconds
    last_method: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        method = _payment_method_for_invoice(
            base_url,
            api_key,
            store_id,
            invoice_id,
            payment_method_id,
        )
        last_method = method
        if _method_payment_count(method) >= expected_payment_count or _method_due_btc(method) < previous_due:
            return method
        time.sleep(2)
    raise RuntimeError(f"BTCPay invoice {invoice_id} did not observe the partial payment: {last_method!r}")


def _scenario_order_id(base_order_id: str, scenario: dict[str, Any]) -> str | None:
    if scenario["kind"] == "payment_request":
        return None
    return f"{base_order_id}-{scenario['suffix']}"


def _scenario_metadata(base_url: str, base_order_id: str, scenario: dict[str, Any]) -> dict[str, Any]:
    metadata = dict(scenario["metadata"])
    metadata["kassiberRegtestScenario"] = scenario["kind"]
    metadata["kassiberRegtestOrderBase"] = base_order_id
    order_url = metadata.get("orderUrl")
    if isinstance(order_url, str) and order_url.startswith("/"):
        metadata["orderUrl"] = base_url.rstrip("/") + order_url
    return metadata


def _exercise_btcpay_invoice(
    *,
    base_url: str,
    api_key: str,
    store_id: str,
    payment_method_id: str,
    base_order_id: str,
    scenario: dict[str, Any],
    currency: str,
    payer_wallet: str,
    wait_seconds: int,
) -> dict[str, Any]:
    kind = str(scenario["kind"])
    order_id = _scenario_order_id(base_order_id, scenario)
    metadata = _scenario_metadata(base_url, base_order_id, scenario)
    invoice_currency = str(scenario.get("currency") or currency)
    invoice = _create_or_get_invoice(
        base_url,
        api_key,
        store_id,
        order_id=order_id,
        amount=str(scenario["amount"]),
        currency=invoice_currency,
        metadata=metadata,
        metadata_key="kassiberRegtestScenario",
        metadata_value=kind,
    )
    invoice_id = str(invoice["id"])
    payment_txids: list[str] = []
    settled_invoice = invoice
    if not _invoice_is_settled(invoice):
        payment_plan = tuple(scenario.get("payment_plan") or ("remaining",))
        method = {}
        for index, planned_amount in enumerate(payment_plan):
            method = _payment_method_for_invoice(
                base_url,
                api_key,
                store_id,
                invoice_id,
                payment_method_id,
            )
            destination = str(method.get("destination") or "")
            due_btc = _method_due_btc(method)
            if not destination or due_btc <= 0:
                raise RuntimeError(f"BTCPay invoice {invoice_id} did not expose a payable on-chain destination")
            if str(planned_amount) == "remaining":
                amount_btc = due_btc
            else:
                amount_btc = min(Decimal(str(planned_amount)), due_btc)
            payment_txids.append(
                _pay_regtest_invoice_from_core(destination, _format_btc_amount(amount_btc), payer_wallet)
            )
            if index < len(payment_plan) - 1:
                method = _wait_for_payment_method_update(
                    base_url=base_url,
                    api_key=api_key,
                    store_id=store_id,
                    invoice_id=invoice_id,
                    payment_method_id=payment_method_id,
                    previous_due=due_btc,
                    expected_payment_count=len(payment_txids),
                    deadline_seconds=wait_seconds,
                )
        settled_invoice = _wait_for_invoice_settlement(
            base_url,
            api_key,
            store_id,
            invoice_id,
            deadline_seconds=wait_seconds,
        )
    return {
        "scenario": kind,
        "invoice_id": invoice_id,
        "invoice_status": settled_invoice.get("status"),
        "invoice_order_id": order_id,
        "invoice_amount": str(scenario["amount"]),
        "invoice_currency": invoice_currency,
        "payment_request_id": metadata.get("paymentRequestId"),
        "order_url": metadata.get("orderUrl"),
        "payment_txid": payment_txids[0] if payment_txids else None,
        "payment_txids": payment_txids,
        "payment_count": len(payment_txids),
    }


def _run_kassiber(args: list[str], *, token: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", "kassiber", *args],
        input=token,
        text=True,
        capture_output=True,
        check=False,
    )


def _run_kassiber_checked(args: list[str], *, token: str | None = None) -> Any:
    completed = _run_kassiber(args, token=token)
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr or completed.stdout)
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"Kassiber did not return JSON: {completed.stdout}") from exc
    if not isinstance(payload, dict):
        raise RuntimeError(f"Unexpected Kassiber response: {payload!r}")
    return payload.get("data")


def _require_mapping(value: Any, description: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RuntimeError(f"Expected {description} to be an object, got {value!r}")
    return value


def _require_list(value: Any, description: str) -> list[Any]:
    if not isinstance(value, list):
        raise RuntimeError(f"Expected {description} to be a list, got {value!r}")
    return value


def _first_match(items: list[Any], description: str, predicate) -> dict[str, Any]:
    for item in items:
        if isinstance(item, dict) and predicate(item):
            return item
    raise RuntimeError(f"Could not find {description}")


def _existing_document_by_external_ref(
    *,
    common: list[str],
    workspace: str,
    profile: str,
    external_ref: str,
) -> dict[str, Any] | None:
    documents = _require_list(
        _run_kassiber_checked(
            [
                *common,
                "documents",
                "list",
                "--workspace",
                workspace,
                "--profile",
                profile,
                "--limit",
                "500",
            ]
        ),
        "external documents",
    )
    for document in documents:
        if isinstance(document, dict) and str(document.get("external_ref") or "") == external_ref:
            return document
    return None


def _create_or_get_payment_request_document(
    *,
    common: list[str],
    workspace: str,
    profile: str,
    payment_request_id: str,
    invoice_currency: str,
    invoice_amount: str,
) -> dict[str, Any]:
    existing = _existing_document_by_external_ref(
        common=common,
        workspace=workspace,
        profile=profile,
        external_ref=payment_request_id,
    )
    if existing is not None:
        return existing

    document_args = [
        *common,
        "documents",
        "create",
        "--workspace",
        workspace,
        "--profile",
        profile,
        "--type",
        "invoice",
        "--label",
        "BTCPay regtest membership invoice",
        "--external-ref",
        payment_request_id,
        "--issuer",
        DEFAULT_STORE_NAME,
        "--counterparty",
        "Kassiber Regtest Member",
        "--notes",
        "Synthetic regtest document keyed by BTCPay payment request id.",
    ]
    if invoice_currency and invoice_currency != "BTC" and invoice_amount:
        document_args.extend(["--fiat-currency", invoice_currency, "--fiat-value", invoice_amount])
    return _require_mapping(
        _run_kassiber_checked(document_args),
        "created commercial document",
    )


def _ensure_kassiber_tag(
    *,
    common: list[str],
    workspace: str,
    profile: str,
    code: str,
    label: str,
) -> None:
    tags = _require_list(
        _run_kassiber_checked(
            [
                *common,
                "metadata",
                "tags",
                "list",
                "--workspace",
                workspace,
                "--profile",
                profile,
            ]
        ),
        "Kassiber tags",
    )
    if any(isinstance(tag, dict) and str(tag.get("code") or "") == code for tag in tags):
        return
    created = _run_kassiber(
        [
            *common,
            "metadata",
            "tags",
            "create",
            "--workspace",
            workspace,
            "--profile",
            profile,
            "--code",
            code,
            "--label",
            label,
        ]
    )
    if created.returncode != 0:
        try:
            payload = json.loads(created.stdout)
            error_code = payload.get("error", {}).get("code") if isinstance(payload, dict) else None
        except json.JSONDecodeError:
            error_code = None
        if error_code != "conflict":
            raise RuntimeError(created.stderr or created.stdout)


def _tag_transaction_for_demo(
    *,
    common: list[str],
    workspace: str,
    profile: str,
    transaction_id: str,
) -> list[dict[str, Any]]:
    applied = []
    for code, label in DEMO_COMMERCIAL_TAGS:
        _ensure_kassiber_tag(common=common, workspace=workspace, profile=profile, code=code, label=label)
        applied.append(
            _require_mapping(
                _run_kassiber_checked(
                    [
                        *common,
                        "metadata",
                        "tags",
                        "add",
                        "--workspace",
                        workspace,
                        "--profile",
                        profile,
                        "--transaction",
                        transaction_id,
                        "--tag",
                        code,
                        "--reason",
                        "BTCPay regtest commercial reconciliation seed.",
                    ]
                ),
                f"applied tag {code}",
            )
        )
    return applied


def _ensure_kassiber_book(data_root: Path, *, workspace: str, profile: str) -> None:
    common = ["--data-root", str(data_root), "--machine"]
    _run_kassiber([*common, "init"])
    _run_kassiber([*common, "workspaces", "create", workspace])
    _run_kassiber(
        [
            *common,
            "profiles",
            "create",
            "--workspace",
            workspace,
            "--fiat-currency",
            "EUR",
            profile,
        ]
    )


def _configure_kassiber(
    *,
    data_root: Path,
    base_url: str,
    api_key: str,
    store_id: str,
    backend_name: str,
    wallet_label: str,
    payment_method_id: str,
    workspace: str,
    profile: str,
) -> None:
    common = ["--data-root", str(data_root), "--machine"]
    _ensure_kassiber_book(data_root, workspace=workspace, profile=profile)
    create_backend = _run_kassiber(
        [
            *common,
            "backends",
            "create",
            backend_name,
            "--kind",
            "btcpay",
            "--url",
            base_url,
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--token-stdin",
        ],
        token=api_key,
    )
    if create_backend.returncode != 0:
        update_backend = _run_kassiber(
            [
                *common,
                "backends",
                "update",
                backend_name,
                "--kind",
                "btcpay",
                "--url",
                base_url,
                "--chain",
                "bitcoin",
                "--network",
                "regtest",
                "--token-stdin",
            ],
            token=api_key,
        )
        if update_backend.returncode != 0:
            raise RuntimeError(update_backend.stderr or update_backend.stdout)

    wallet_args = [
        *common,
        "wallets",
        "create",
        "--workspace",
        workspace,
        "--profile",
        profile,
        "--label",
        wallet_label,
        "--kind",
        "custom",
        "--backend",
        backend_name,
        "--chain",
        "bitcoin",
        "--network",
        "regtest",
        "--store-id",
        store_id,
        "--payment-method-id",
        payment_method_id,
    ]
    create_wallet = _run_kassiber(wallet_args)
    if create_wallet.returncode == 0:
        return
    update_wallet = _run_kassiber(
        [
            *common,
            "wallets",
            "update",
            "--workspace",
            workspace,
            "--profile",
            profile,
            "--wallet",
            wallet_label,
            "--backend",
            backend_name,
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--store-id",
            store_id,
            "--payment-method-id",
            payment_method_id,
        ]
    )
    if update_wallet.returncode != 0:
        raise RuntimeError(update_wallet.stderr or update_wallet.stdout)


def _exercise_btcpay_commercial_reconciliation(
    *,
    data_root: Path,
    invoice_results: list[dict[str, Any]],
    workspace: str,
    profile: str,
) -> dict[str, Any]:
    common = ["--data-root", str(data_root), "--machine"]
    payment_request_invoice = _first_match(
        invoice_results,
        "payment-request invoice result",
        lambda invoice: invoice.get("scenario") == "payment_request",
    )
    payment_request_id = str(payment_request_invoice.get("payment_request_id") or "")
    if not payment_request_id:
        raise RuntimeError("Payment-request invoice did not expose a payment_request_id")

    invoice_currency = str(payment_request_invoice.get("invoice_currency") or "")
    invoice_amount = str(payment_request_invoice.get("invoice_amount") or "")
    document = _create_or_get_payment_request_document(
        common=common,
        workspace=workspace,
        profile=profile,
        payment_request_id=payment_request_id,
        invoice_currency=invoice_currency,
        invoice_amount=invoice_amount,
    )

    suggest = _require_mapping(
        _run_kassiber_checked(
            [
                *common,
                "btcpay",
                "provenance",
                "suggest",
                "--workspace",
                workspace,
                "--profile",
                profile,
                "--limit",
                "100",
            ]
        ),
        "BTCPay provenance suggestions",
    )
    suggestions = _require_list(suggest.get("suggestions"), "BTCPay provenance suggestions")
    document_id = str(document.get("id") or "")
    document_link = _first_match(
        suggestions,
        "payment-request document to BTCPay invoice link",
        lambda link: link.get("link_type") == "document_btcpay"
        and link.get("document_id") == document_id
        and link.get("payment_request_id") == payment_request_id,
    )
    payment_link = _first_match(
        suggestions,
        "payment-request BTCPay payment to transaction link",
        lambda link: link.get("link_type") == "btcpay_payment_transaction"
        and link.get("document_id") == document_id
        and link.get("payment_request_id") == payment_request_id
        and bool(link.get("transaction_id")),
    )
    reviewed = _require_mapping(
        _run_kassiber_checked(
            [
                *common,
                "btcpay",
                "provenance",
                "review",
                "--workspace",
                workspace,
                "--profile",
                profile,
                "--link",
                str(payment_link["id"]),
                "--state",
                "reviewed",
                "--reconciliation-state",
                "matched",
                "--commercial-kind",
                "income",
                "--notes",
                "Reviewed by BTCPay regtest reconciliation proof.",
            ]
        ),
        "reviewed BTCPay provenance link",
    )
    if not reviewed.get("applied_to_transaction"):
        raise RuntimeError(f"Reviewed BTCPay link did not apply pricing to the transaction: {reviewed!r}")

    reviewed_links = _require_list(
        _run_kassiber_checked(
            [
                *common,
                "btcpay",
                "provenance",
                "links",
                "--workspace",
                workspace,
                "--profile",
                profile,
                "--state",
                "reviewed",
                "--limit",
                "100",
            ]
        ),
        "reviewed BTCPay provenance links",
    )
    subledger = _require_list(
        _run_kassiber_checked(
            [
                *common,
                "reports",
                "commercial-subledger",
                "--workspace",
                workspace,
                "--profile",
                profile,
            ]
        ),
        "commercial subledger",
    )
    subledger_row = _first_match(
        subledger,
        "reviewed payment-request commercial subledger row",
        lambda row: row.get("transaction_id") == payment_link.get("transaction_id")
        and row.get("payment_request_id") == payment_request_id,
    )
    if subledger_row.get("pricing_source_kind") != "btcpay_payment":
        raise RuntimeError(f"Subledger row did not use BTCPay payment pricing: {subledger_row!r}")
    if subledger_row.get("commercial_kind") != "income":
        raise RuntimeError(f"Subledger row did not classify reviewed payment as income: {subledger_row!r}")
    applied_tags = _tag_transaction_for_demo(
        common=common,
        workspace=workspace,
        profile=profile,
        transaction_id=str(payment_link["transaction_id"]),
    )

    return {
        "document_id": document_id,
        "document_external_ref": payment_request_id,
        "document_link_id": document_link["id"],
        "reviewed_link_id": reviewed["id"],
        "reviewed_links": len(reviewed_links),
        "suggested_links": len(suggestions),
        "transaction_id": payment_link["transaction_id"],
        "transaction_txid": payment_link.get("transaction_external_id") or "",
        "invoice_id": subledger_row.get("invoice_id") or payment_request_invoice.get("invoice_id"),
        "payment_request_id": payment_request_id,
        "origin_kind": subledger_row.get("origin_kind") or "",
        "origin_label": subledger_row.get("origin_label") or "",
        "pricing_source_kind": subledger_row["pricing_source_kind"],
        "fiat_currency": subledger_row.get("fiat_currency") or "",
        "fiat_value_exact": subledger_row.get("fiat_value_exact") or "",
        "commercial_kind": subledger_row["commercial_kind"],
        "transaction_tags": [tag["tag"] for tag in applied_tags],
        "subledger_rows": len(subledger),
    }


def _exercise_kassiber_btcpay_sync(
    *,
    data_root: Path,
    backend_name: str,
    wallet_label: str,
    store_id: str,
    payment_method_id: str,
    workspace: str,
    profile: str,
    invoice_results: list[dict[str, Any]],
) -> dict[str, Any]:
    common = ["--data-root", str(data_root), "--machine"]
    wallet_sync = _run_kassiber_checked(
        [
            *common,
            "wallets",
            "sync-btcpay",
            "--workspace",
            workspace,
            "--profile",
            profile,
            "--wallet",
            wallet_label,
            "--backend",
            backend_name,
            "--store-id",
            store_id,
            "--payment-method-id",
            payment_method_id,
        ]
    )
    provenance_sync = _run_kassiber_checked(
        [
            *common,
            "btcpay",
            "provenance",
            "sync",
            "--workspace",
            workspace,
            "--profile",
            profile,
            "--backend",
            backend_name,
            "--store-id",
            store_id,
        ]
    )
    provenance_list = _run_kassiber_checked(
        [
            *common,
            "btcpay",
            "provenance",
            "list",
            "--workspace",
            workspace,
            "--profile",
            profile,
            "--record-type",
            "payment",
            "--limit",
            "50",
        ]
    )
    records = provenance_list if isinstance(provenance_list, list) else None
    origin_kinds = sorted(
        {
            str(record.get("origin_kind"))
            for record in (records or [])
            if isinstance(record, dict) and record.get("origin_kind")
        }
    )
    payment_txids = sorted(
        {
            str(record.get("txid"))
            for record in (records or [])
            if isinstance(record, dict) and record.get("txid")
        }
    )
    commercial_reconciliation = _exercise_btcpay_commercial_reconciliation(
        data_root=data_root,
        invoice_results=invoice_results,
        workspace=workspace,
        profile=profile,
    )
    return {
        "wallet_sync": wallet_sync,
        "provenance_sync": provenance_sync,
        "provenance_payment_records": len(records) if isinstance(records, list) else None,
        "provenance_origin_kinds": origin_kinds,
        "provenance_payment_txids": payment_txids,
        "commercial_reconciliation": commercial_reconciliation,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Seed a disposable BTCPay regtest store.")
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--user", default=DEFAULT_USER)
    parser.add_argument("--password", default=DEFAULT_PASSWORD)
    parser.add_argument("--store-name", default=DEFAULT_STORE_NAME)
    parser.add_argument("--payment-method-id", default=DEFAULT_PAYMENT_METHOD_ID)
    parser.add_argument("--workspace", default=DEFAULT_WORKSPACE)
    parser.add_argument("--profile", default=DEFAULT_PROFILE)
    parser.add_argument("--kassiber-data-root")
    parser.add_argument("--backend-name", default=DEFAULT_BACKEND_NAME)
    parser.add_argument("--wallet-label", default=DEFAULT_WALLET_LABEL)
    parser.add_argument(
        "--lightning-connection-string",
        default=os.environ.get("KASSIBER_REGTEST_BTCPAY_LIGHTNING_CONNECTION_STRING"),
    )
    parser.add_argument("--exercise-invoice", action="store_true")
    parser.add_argument("--invoice-order-id", default=DEFAULT_ORDER_ID)
    parser.add_argument("--invoice-amount", default="0.00021000")
    parser.add_argument("--invoice-currency", default="BTC")
    parser.add_argument("--payer-wallet", default="kassiber-btcpay-payer")
    parser.add_argument("--json-output")
    parser.add_argument("--wait-seconds", type=int, default=180)
    args = parser.parse_args(argv)

    _wait_for_btcpay(args.base_url, deadline_seconds=args.wait_seconds)
    _warm_core_from_env()
    _ensure_user(args.base_url, args.user, args.password)
    store = _ensure_store(args.base_url, args.user, args.password, args.store_name)
    store_id = str(store["id"])
    generated_wallet = _ensure_wallet(
        args.base_url,
        args.user,
        args.password,
        store_id,
        args.payment_method_id,
    )
    lightning_configured = _ensure_lightning(
        args.base_url,
        args.user,
        args.password,
        store_id,
        args.lightning_connection_string,
    )
    api_key = _create_api_key(args.base_url, args.user, args.password, store_id)
    if args.kassiber_data_root:
        _configure_kassiber(
            data_root=Path(args.kassiber_data_root),
            base_url=args.base_url,
            api_key=api_key,
            store_id=store_id,
            backend_name=args.backend_name,
            wallet_label=args.wallet_label,
            payment_method_id=args.payment_method_id,
            workspace=args.workspace,
            profile=args.profile,
        )
    invoice_results: list[dict[str, Any]] = []
    btcpay_exercise: dict[str, Any] | None = None
    if args.exercise_invoice:
        if not args.kassiber_data_root:
            raise RuntimeError("--exercise-invoice requires --kassiber-data-root")
        first_scenario = dict(REGTEST_INVOICE_SCENARIOS[0])
        first_scenario["amount"] = args.invoice_amount
        scenarios = (first_scenario, *REGTEST_INVOICE_SCENARIOS[1:])
        for scenario in scenarios:
            invoice_results.append(
                _exercise_btcpay_invoice(
                    base_url=args.base_url,
                    api_key=api_key,
                    store_id=store_id,
                    payment_method_id=args.payment_method_id,
                    base_order_id=args.invoice_order_id,
                    scenario=scenario,
                    currency=args.invoice_currency,
                    payer_wallet=args.payer_wallet,
                    wait_seconds=args.wait_seconds,
                )
            )
        kassiber_sync = _exercise_kassiber_btcpay_sync(
            data_root=Path(args.kassiber_data_root),
            backend_name=args.backend_name,
            wallet_label=args.wallet_label,
            store_id=store_id,
            payment_method_id=args.payment_method_id,
            workspace=args.workspace,
            profile=args.profile,
            invoice_results=invoice_results,
        )
        btcpay_exercise = {
            "invoice_count": len(invoice_results),
            "scenarios": [invoice["scenario"] for invoice in invoice_results],
            "settled_invoice_count": sum(
                1 for invoice in invoice_results if str(invoice.get("invoice_status") or "") == "Settled"
            ),
            "payment_txids": [
                txid for invoice in invoice_results for txid in invoice.get("payment_txids", [])
            ],
            "kassiber": kassiber_sync,
        }

    payload = {
        "base_url": args.base_url,
        "user": args.user,
        "password": args.password,
        "store_id": store_id,
        "store_name": args.store_name,
        "payment_method_id": args.payment_method_id,
        "api_key": api_key,
        "generated_wallet": generated_wallet,
        "lightning_configured": lightning_configured,
        "lightning_connection_string": args.lightning_connection_string if lightning_configured else None,
        "backend": args.backend_name if args.kassiber_data_root else None,
        "wallet": args.wallet_label if args.kassiber_data_root else None,
        "invoice": invoice_results[0] if invoice_results else None,
        "invoices": invoice_results,
        "btcpay_regtest": btcpay_exercise,
    }
    if args.json_output:
        output = Path(args.json_output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.chmod(output, 0o600)
    else:
        print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
