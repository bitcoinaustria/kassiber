"""BTCPay Greenfield API fetcher for confirmed on-chain wallet transactions.

The public entry point is `fetch_btcpay_records(backend, store_id, ...)`,
which hits `GET /api/v1/stores/{storeId}/payment-methods/{paymentMethodId}/wallet/transactions`,
pages through the result with `skip`/`limit`, requests confirmed rows only,
and returns records in the same shape `kassiber.importers.normalize_btcpay_record`
already produces. That lets the CLI coordinator reuse the normal BTCPay
import path for transaction insertion plus note/tag metadata application.
"""

from __future__ import annotations

import datetime as _dt
import json
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .backends import backend_timeout, backend_value
from .errors import AppError
from .importers import normalize_btcpay_record, parse_btcpay_labels


DEFAULT_PAYMENT_METHOD_ID = "BTC-CHAIN"
DEFAULT_PAGE_SIZE = 100
DEFAULT_STATUS_FILTER = "Confirmed"
MAX_PAGES = 10_000


def fetch_btcpay_records(
    backend,
    store_id,
    payment_method_id=DEFAULT_PAYMENT_METHOD_ID,
    page_size=DEFAULT_PAGE_SIZE,
    opener=None,
):
    if not store_id:
        raise AppError("BTCPay store id is required", code="validation")
    require_wallet_history_payment_method(payment_method_id)
    base = backend_value(backend, "url")
    if not base:
        raise AppError("BTCPay instance is missing 'url'", code="config_error")
    token = backend_value(backend, "token")
    if not token:
        raise AppError(
            "BTCPay instance is missing 'token' (api key)",
            code="config_error",
            hint="Store the api key with `kassiber backends update --token <key>` or KASSIBER_BACKEND_<NAME>_TOKEN.",
        )
    if page_size <= 0:
        raise AppError("BTCPay page_size must be positive", code="validation")
    timeout = backend_timeout(backend)
    http_opener = opener or urlrequest.build_opener()
    records = []
    skip = 0
    page_count = 0
    while True:
        if page_count >= MAX_PAGES:
            raise AppError(
                f"BTCPay sync exceeded {MAX_PAGES} pages; aborting for safety",
                code="config_error",
            )
        url = _build_list_url(base, store_id, payment_method_id, skip, page_size)
        page = _http_get_json(
            http_opener,
            url,
            token,
            timeout,
            permission_hint=(
                "Greenfield wallet endpoints currently require the "
                "`btcpay.store.canmodifystoresettings` permission."
            ),
        )
        if not isinstance(page, list):
            raise AppError(
                f"BTCPay response for {url} was not a JSON array",
                code="protocol_error",
            )
        for tx in page:
            if _is_confirmed_transaction(tx):
                records.append(_to_record(tx, payment_method_id))
        page_count += 1
        if len(page) < page_size:
            break
        skip += page_size
    return records


def probe_btcpay_wallet(
    backend,
    store_id,
    payment_method_id=DEFAULT_PAYMENT_METHOD_ID,
    opener=None,
):
    """Validate one BTCPay wallet-history request without walking the paginator."""

    if not store_id:
        raise AppError("BTCPay store id is required", code="validation")
    require_wallet_history_payment_method(payment_method_id)
    base = backend_value(backend, "url")
    if not base:
        raise AppError("BTCPay instance is missing 'url'", code="config_error")
    token = backend_value(backend, "token")
    if not token:
        raise AppError(
            "BTCPay instance is missing 'token' (api key)",
            code="config_error",
            hint="Store the api key with `kassiber backends update --token <key>` or KASSIBER_BACKEND_<NAME>_TOKEN.",
        )
    timeout = backend_timeout(backend)
    http_opener = opener or urlrequest.build_opener()
    url = _build_list_url(base, store_id, payment_method_id, 0, 1)
    page = _http_get_json(
        http_opener,
        url,
        token,
        timeout,
        permission_hint=(
            "Greenfield wallet endpoints currently require the "
            "`btcpay.store.canmodifystoresettings` permission."
        ),
    )
    if not isinstance(page, list):
        raise AppError(
            f"BTCPay response for {url} was not a JSON array",
            code="protocol_error",
        )
    return {"checked": True, "rows_seen": len(page)}


def discover_btcpay_wallet_sources(backend, opener=None):
    """Return stores and enabled on-chain payment methods for setup forms.

    The discovery path intentionally does not request payment-method config,
    because those payloads may contain wallet material. Kassiber only needs
    the stable store id and payment method id to configure a confirmed wallet
    history sync.
    """

    base = backend_value(backend, "url")
    if not base:
        raise AppError("BTCPay instance is missing 'url'", code="config_error")
    token = backend_value(backend, "token")
    if not token:
        raise AppError(
            "BTCPay instance is missing 'token' (api key)",
            code="config_error",
            hint="Enter a Greenfield API key for this BTCPay instance.",
        )
    timeout = backend_timeout(backend)
    http_opener = opener or urlrequest.build_opener()
    stores_url = _build_stores_url(base)
    raw_stores = _http_get_json(
        http_opener,
        stores_url,
        token,
        timeout,
        permission_hint=(
            "Grant the API key access to view stores, or enter the store ID manually."
        ),
    )
    if not isinstance(raw_stores, list):
        raise AppError(
            f"BTCPay response for {stores_url} was not a JSON array",
            code="protocol_error",
        )

    stores = []
    payment_methods = []
    for raw_store in raw_stores:
        store = _normalize_store(raw_store)
        stores.append(store)
        methods_url = _build_payment_methods_url(base, store["id"])
        raw_methods = _http_get_json(
            http_opener,
            methods_url,
            token,
            timeout,
            permission_hint=(
                "Grant the API key store-settings access to inspect enabled payment methods."
            ),
        )
        if not isinstance(raw_methods, list):
            raise AppError(
                f"BTCPay response for {methods_url} was not a JSON array",
                code="protocol_error",
            )
        for raw_method in raw_methods:
            method = _normalize_payment_method(store["id"], raw_method)
            if method is not None:
                payment_methods.append(method)
    return {"stores": stores, "payment_methods": payment_methods}


def _build_list_url(base, store_id, payment_method_id, skip, limit):
    base = base.rstrip("/")
    store_q = urlparse.quote(store_id, safe="")
    payment_q = urlparse.quote(payment_method_id, safe="")
    query = urlparse.urlencode(
        {
            "statusFilter": DEFAULT_STATUS_FILTER,
            "skip": str(skip),
            "limit": str(limit),
        }
    )
    return f"{base}/api/v1/stores/{store_q}/payment-methods/{payment_q}/wallet/transactions?{query}"


def _build_stores_url(base):
    return f"{base.rstrip('/')}/api/v1/stores"


def _build_payment_methods_url(base, store_id):
    store_q = urlparse.quote(store_id, safe="")
    query = urlparse.urlencode({"onlyEnabled": "true"})
    return f"{base.rstrip('/')}/api/v1/stores/{store_q}/payment-methods?{query}"


def _http_get_json(opener, url, token, timeout, *, permission_hint=None):
    request = urlrequest.Request(
        url,
        headers={
            "Accept": "application/json",
            "Authorization": f"token {token}",
        },
    )
    try:
        with opener.open(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        if exc.code == 401:
            raise AppError(
                f"BTCPay rejected the API key (HTTP 401) for {url}",
                code="auth_error",
                hint="Check that `token` on the backend is current and not revoked.",
            ) from exc
        if exc.code == 403:
            raise AppError(
                f"BTCPay API key is missing the required permission (HTTP 403) for {url}",
                code="auth_error",
                hint=permission_hint
                or "Check the permissions granted to the Greenfield API key.",
            ) from exc
        if exc.code == 404:
            raise AppError(
                f"BTCPay store or payment method not found (HTTP 404): {url}",
                code="not_found",
                hint="Verify --store-id and --payment-method-id (default BTC-CHAIN).",
            ) from exc
        raise AppError(
            f"HTTP {exc.code} from BTCPay for {url}: {detail[:200]}",
            code="protocol_error",
        ) from exc
    except urlerror.URLError as exc:
        raise AppError(
            f"Failed to reach BTCPay server {url}: {exc.reason}",
            code="network_error",
            retryable=True,
        ) from exc


def _normalize_store(raw_store):
    if not isinstance(raw_store, dict):
        raise AppError("BTCPay store record was not a JSON object", code="protocol_error")
    store_id = raw_store.get("id") or raw_store.get("storeId")
    if not store_id:
        raise AppError("BTCPay store record is missing 'id'", code="protocol_error")
    label = raw_store.get("name") or raw_store.get("label") or store_id
    return {
        "id": str(store_id),
        "name": str(label),
        "default_currency": raw_store.get("defaultCurrency"),
    }


def _normalize_payment_method(store_id, raw_method):
    if not isinstance(raw_method, dict):
        raise AppError(
            "BTCPay payment-method record was not a JSON object",
            code="protocol_error",
        )
    method_id = (
        raw_method.get("paymentMethodId")
        or raw_method.get("paymentMethod")
        or raw_method.get("id")
    )
    if not method_id:
        crypto_code = raw_method.get("cryptoCode") or raw_method.get("currency")
        payment_type = str(raw_method.get("paymentType") or "").lower()
        if crypto_code and ("chain" in payment_type or "onchain" in payment_type):
            method_id = f"{crypto_code}-CHAIN"
    if not method_id:
        return None
    method_id = str(method_id)
    sync_supported = _is_wallet_history_payment_method(method_id)
    return {
        "store_id": store_id,
        "payment_method_id": method_id,
        "label": str(raw_method.get("name") or raw_method.get("label") or method_id),
        "enabled": bool(raw_method.get("enabled", True)),
        "sync_supported": sync_supported,
    }


def _is_wallet_history_payment_method(payment_method_id):
    normalized = payment_method_id.upper()
    if normalized.endswith("-LN") or "LIGHTNING" in normalized or "LNURL" in normalized:
        return False
    return normalized.endswith("-CHAIN") or "ONCHAIN" in normalized or "-" not in normalized


def require_wallet_history_payment_method(payment_method_id):
    value = str(payment_method_id or "")
    if _is_wallet_history_payment_method(value):
        return value
    raise AppError(
        f"BTCPay payment method '{payment_method_id}' is not available through wallet-history sync",
        code="validation",
        hint=(
            "Use an on-chain method such as BTC-CHAIN or LBTC-CHAIN. "
            "BTC-LN requires invoice/settlement provenance ingest before Kassiber can match it to a settlement wallet."
        ),
    )


def _is_confirmed_transaction(tx):
    if not isinstance(tx, dict):
        raise AppError("BTCPay transaction record was not a JSON object", code="protocol_error")
    confirmations = tx.get("confirmations")
    if confirmations not in (None, ""):
        try:
            return int(confirmations) > 0
        except (TypeError, ValueError) as exc:
            raise AppError(
                f"Invalid BTCPay confirmations value '{confirmations}'",
                code="protocol_error",
            ) from exc
    status = str(tx.get("status") or "").strip().lower()
    if status:
        return status == "confirmed"
    return True


def _to_record(tx, payment_method_id):
    currency = payment_method_id.split("-", 1)[0].upper() if payment_method_id else "BTC"
    timestamp = tx.get("timestamp")
    if timestamp is None:
        raise AppError("BTCPay transaction is missing 'timestamp'", code="protocol_error")
    occurred_at = _unix_to_iso(timestamp)
    csv_shaped = {
        "TransactionId": tx.get("transactionHash") or "",
        "Timestamp": occurred_at,
        "confirmed_at": occurred_at,
        "Currency": currency,
        "Amount": str(tx.get("amount") if tx.get("amount") is not None else "0"),
        "Comment": tx.get("comment") or "",
        "Labels": parse_btcpay_labels(tx.get("labels")),
    }
    return normalize_btcpay_record(csv_shaped)


def _unix_to_iso(ts):
    try:
        value = int(ts)
    except (TypeError, ValueError):
        try:
            value = int(float(ts))
        except (TypeError, ValueError) as exc:
            raise AppError(
                f"Invalid BTCPay timestamp '{ts}'",
                code="protocol_error",
            ) from exc
    return _dt.datetime.fromtimestamp(value, tz=_dt.timezone.utc).isoformat().replace("+00:00", "Z")


__all__ = [
    "DEFAULT_PAGE_SIZE",
    "DEFAULT_PAYMENT_METHOD_ID",
    "discover_btcpay_wallet_sources",
    "fetch_btcpay_records",
    "probe_btcpay_wallet",
    "require_wallet_history_payment_method",
]
