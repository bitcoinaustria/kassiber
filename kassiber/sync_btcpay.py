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
import hashlib
import json
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .backends import backend_timeout, backend_value
from .errors import AppError
from .importers import normalize_btcpay_record, parse_btcpay_labels
from .retry import retry_after_seconds_from_http_error


DEFAULT_PAYMENT_METHOD_ID = "BTC-CHAIN"
DEFAULT_PAGE_SIZE = 100
DEFAULT_STATUS_FILTER = "Confirmed"
MAX_PAGES = 10_000
INCREMENTAL_UNCHANGED_PAGE_WINDOW = 5
INCREMENTAL_DEEP_AUDIT_PAGES = 1

# Kassiber currently understands wallet-history sync for Bitcoin and Liquid
# on-chain only. Adding a new entry here is the single step required to
# extend support — both the desktop discovery UI and the daemon validation
# paths read from this allowlist.
WALLET_HISTORY_PAYMENT_METHOD_IDS = frozenset({"BTC-CHAIN", "LBTC-CHAIN"})


def fetch_btcpay_records(
    backend,
    store_id,
    payment_method_id=DEFAULT_PAYMENT_METHOD_ID,
    page_size=DEFAULT_PAGE_SIZE,
    opener=None,
    checkpoint=None,
    metadata=None,
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
            hint="Store the api key with `kassiber backends update --token-stdin` or `--token-fd FD`.",
        )
    if page_size <= 0:
        raise AppError("BTCPay page_size must be positive", code="validation")
    timeout = backend_timeout(backend)
    http_opener = opener or urlrequest.build_opener()
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    previous_pages = checkpoint.get("btcpay_pages") or {}
    previous_pagination = checkpoint.get("btcpay_pagination") or {}

    def fetch_page(skip):
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
        return page

    records, next_pages, page_metadata = _fetch_incremental_pages(
        fetch_page=fetch_page,
        page_size=page_size,
        previous_pages=previous_pages,
        previous_pagination=previous_pagination,
        fingerprint_fn=_page_fingerprint,
        stable_ids_fn=_page_stable_ids,
        normalize_page=lambda page: [
            _to_record(tx, payment_method_id)
            for tx in page
            if _is_confirmed_transaction(tx)
        ],
        max_pages_message=f"BTCPay sync exceeded {MAX_PAGES} pages; aborting for safety",
    )
    if metadata is not None:
        metadata.update(
            {
                "btcpay_pages": _sorted_page_map(next_pages),
                "btcpay_pagination": page_metadata["pagination"],
                "pages_fetched": page_metadata["pages_fetched"],
                "stopped_by_known_page": page_metadata["stopped_by_known_page"],
                "stop_reason": page_metadata["stop_reason"],
                "deep_audit": page_metadata.get("deep_audit"),
                "changed_pages": page_metadata["changed_pages"],
            }
        )
    return records


def fetch_btcpay_invoice_provenance(
    backend,
    store_id,
    *,
    page_size=DEFAULT_PAGE_SIZE,
    opener=None,
    checkpoint=None,
    metadata=None,
):
    """Fetch invoice/payment provenance without importing wallet balances."""

    if not store_id:
        raise AppError("BTCPay store id is required", code="validation")
    base = backend_value(backend, "url")
    if not base:
        raise AppError("BTCPay instance is missing 'url'", code="config_error")
    token = backend_value(backend, "token")
    if not token:
        raise AppError(
            "BTCPay instance is missing 'token' (api key)",
            code="config_error",
            hint="Store the api key with `kassiber backends update --token-stdin` or `--token-fd FD`.",
        )
    if page_size <= 0:
        raise AppError("BTCPay page_size must be positive", code="validation")
    timeout = backend_timeout(backend)
    http_opener = opener or urlrequest.build_opener()
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    previous_pages = checkpoint.get("btcpay_invoice_pages") or {}
    previous_pagination = checkpoint.get("btcpay_invoice_pagination") or {}

    def fetch_page(skip):
        url = _build_invoices_url(base, store_id, skip, page_size)
        page = _http_get_json(
            http_opener,
            url,
            token,
            timeout,
            permission_hint="Grant the API key the BTCPay 'View invoices' permission.",
        )
        if not isinstance(page, list):
            raise AppError(
                f"BTCPay response for {url} was not a JSON array",
                code="protocol_error",
            )
        return page

    invoices, next_pages, page_metadata = _fetch_incremental_pages(
        fetch_page=fetch_page,
        page_size=page_size,
        previous_pages=previous_pages,
        previous_pagination=previous_pagination,
        fingerprint_fn=_invoice_page_fingerprint,
        stable_ids_fn=_invoice_page_stable_ids,
        normalize_page=lambda page: [
            _normalize_invoice_provenance(store_id, invoice) for invoice in page
        ],
        max_pages_message=f"BTCPay invoice sync exceeded {MAX_PAGES} pages; aborting for safety",
    )
    if metadata is not None:
        metadata.update(
            {
                "btcpay_invoice_pages": _sorted_page_map(next_pages),
                "btcpay_invoice_pagination": page_metadata["pagination"],
                "pages_fetched": page_metadata["pages_fetched"],
                "stopped_by_known_page": page_metadata["stopped_by_known_page"],
                "stop_reason": page_metadata["stop_reason"],
                "deep_audit": page_metadata.get("deep_audit"),
                "changed_pages": page_metadata["changed_pages"],
            }
        )
    return invoices


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
            hint="Store the api key with `kassiber backends update --token-stdin` or `--token-fd FD`.",
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


def _build_invoices_url(base, store_id, skip, limit):
    store_q = urlparse.quote(store_id, safe="")
    query = urlparse.urlencode({"skip": str(skip), "take": str(limit)})
    return f"{base.rstrip('/')}/api/v1/stores/{store_q}/invoices?{query}"


def _page_sort_key(item):
    key, _ = item
    try:
        return int(key)
    except (TypeError, ValueError):
        return MAX_PAGES * DEFAULT_PAGE_SIZE


def _sorted_page_map(pages):
    if not isinstance(pages, dict):
        return {}
    return {
        str(key): value
        for key, value in sorted(pages.items(), key=_page_sort_key)
        if isinstance(value, dict)
    }


def _positive_checkpoint_int(value, default, *, maximum):
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return default
    if parsed <= 0:
        return default
    return min(parsed, maximum)


def _page_checkpoint(page, *, fingerprint, stable_ids, page_size):
    return {
        "fingerprint": fingerprint,
        "stable_ids": stable_ids,
        "rows": len(page),
        "page_size": page_size,
    }


def _same_page(previous, *, fingerprint, stable_ids, page_size):
    if not isinstance(previous, dict):
        return False
    previous_stable_ids = previous.get("stable_ids")
    return (
        isinstance(previous_stable_ids, list)
        and previous_stable_ids == stable_ids
        and previous.get("fingerprint") == fingerprint
        and previous.get("page_size") == page_size
    )


def _int_page_key(key):
    try:
        return int(key)
    except (TypeError, ValueError):
        return None


def _prune_pages_after_terminal(pages, terminal_skip):
    if terminal_skip is None:
        return pages
    pruned = {}
    for key, value in pages.items():
        numeric = _int_page_key(key)
        if numeric is None or numeric <= terminal_skip:
            pruned[key] = value
    return pruned


def _fetch_incremental_pages(
    *,
    fetch_page,
    page_size,
    previous_pages,
    previous_pagination,
    fingerprint_fn,
    stable_ids_fn,
    normalize_page,
    max_pages_message,
):
    previous_pages = _sorted_page_map(previous_pages)
    previous_pagination = (
        previous_pagination if isinstance(previous_pagination, dict) else {}
    )
    unchanged_window = _positive_checkpoint_int(
        previous_pagination.get("unchanged_page_window"),
        INCREMENTAL_UNCHANGED_PAGE_WINDOW,
        maximum=100,
    )
    deep_audit_pages = _positive_checkpoint_int(
        previous_pagination.get("deep_audit_pages"),
        INCREMENTAL_DEEP_AUDIT_PAGES,
        maximum=20,
    )
    next_pages = dict(previous_pages)
    records = []
    pages_fetched = 0
    fetched_skips: set[int] = set()
    changed_pages: list[int] = []
    unchanged_pages: list[int] = []
    stopped_by_known_page = False
    stop_reason = "end_of_results"
    terminal_skip: int | None = None
    unchanged_streak = 0
    skip = 0

    def load_page(page_skip):
        nonlocal pages_fetched
        if pages_fetched >= MAX_PAGES:
            raise AppError(max_pages_message, code="config_error")
        page = fetch_page(page_skip)
        pages_fetched += 1
        fetched_skips.add(page_skip)
        return page

    def process_page(page_skip, page):
        nonlocal stopped_by_known_page, unchanged_streak
        page_key = str(page_skip)
        stable_ids = stable_ids_fn(page)
        fingerprint = fingerprint_fn(page)
        previous = previous_pages.get(page_key)
        unchanged = _same_page(
            previous,
            fingerprint=fingerprint,
            stable_ids=stable_ids,
            page_size=page_size,
        )
        if unchanged:
            next_pages[page_key] = previous
            stopped_by_known_page = True
            unchanged_pages.append(page_skip)
            unchanged_streak += 1
            return False
        records.extend(normalize_page(page))
        next_pages[page_key] = _page_checkpoint(
            page,
            fingerprint=fingerprint,
            stable_ids=stable_ids,
            page_size=page_size,
        )
        changed_pages.append(page_skip)
        unchanged_streak = 0
        return True

    while True:
        page = load_page(skip)
        process_page(skip, page)
        if len(page) < page_size:
            terminal_skip = skip
            stop_reason = "end_of_results"
            break
        if previous_pages and unchanged_streak >= unchanged_window:
            stop_reason = "unchanged_page_window"
            break
        skip += page_size

    deep_audit = None
    next_deep_audit_skip = None
    if stop_reason == "unchanged_page_window" and deep_audit_pages > 0:
        minimum_deep_skip = skip + page_size
        saved_deep_skip = _int_page_key(previous_pagination.get("next_deep_audit_skip"))
        audit_skip = (
            saved_deep_skip
            if saved_deep_skip is not None and saved_deep_skip >= minimum_deep_skip
            else minimum_deep_skip
        )
        audit_start = audit_skip
        audited = 0
        audit_stop_reason = "deep_audit_window"
        while audited < deep_audit_pages:
            if audit_skip in fetched_skips:
                audit_skip += page_size
                continue
            page = load_page(audit_skip)
            process_page(audit_skip, page)
            audited += 1
            if len(page) < page_size:
                terminal_skip = audit_skip if terminal_skip is None else min(terminal_skip, audit_skip)
                audit_stop_reason = "end_of_results"
                audit_skip = minimum_deep_skip
                break
            audit_skip += page_size
        next_deep_audit_skip = audit_skip
        deep_audit = {
            "start_skip": audit_start,
            "pages": audited,
            "stop_reason": audit_stop_reason,
            "next_skip": next_deep_audit_skip,
        }

    next_pages = _prune_pages_after_terminal(next_pages, terminal_skip)
    pagination = {
        "unchanged_page_window": unchanged_window,
        "deep_audit_pages": deep_audit_pages,
        "last_stop_reason": stop_reason,
        "next_deep_audit_skip": next_deep_audit_skip,
    }
    if deep_audit is not None:
        pagination["last_deep_audit"] = deep_audit
    metadata = {
        "pages_fetched": pages_fetched,
        "stopped_by_known_page": stopped_by_known_page,
        "stop_reason": stop_reason,
        "changed_pages": changed_pages,
        "unchanged_pages": unchanged_pages,
        "deep_audit": deep_audit,
        "pagination": pagination,
    }
    return records, _sorted_page_map(next_pages), metadata


def _stable_transaction_id(tx):
    if not isinstance(tx, dict):
        return ""
    return str(
        tx.get("transactionHash")
        or tx.get("transactionId")
        or tx.get("id")
        or json.dumps(tx, sort_keys=True)
    )


def _page_stable_ids(page):
    return sorted(_stable_transaction_id(tx) for tx in page if isinstance(tx, dict))


def _page_fingerprint_rows(page):
    rows = []
    for tx in page:
        if not isinstance(tx, dict):
            continue
        rows.append(
            {
                "id": _stable_transaction_id(tx),
                "timestamp": tx.get("timestamp"),
                "amount": tx.get("amount"),
                "confirmations": tx.get("confirmations"),
                "status": tx.get("status"),
                "comment": tx.get("comment"),
                "labels": parse_btcpay_labels(tx.get("labels")),
            }
        )
    return sorted(rows, key=lambda row: row["id"])


def _page_fingerprint(page):
    return hashlib.sha256(
        json.dumps(_page_fingerprint_rows(page), sort_keys=True).encode("utf-8")
    ).hexdigest()


def _invoice_stable_id(invoice):
    if not isinstance(invoice, dict):
        return ""
    return str(invoice.get("id") or invoice.get("invoiceId") or json.dumps(invoice, sort_keys=True))


def _invoice_page_stable_ids(page):
    return sorted(_invoice_stable_id(invoice) for invoice in page if isinstance(invoice, dict))


def _invoice_page_fingerprint_rows(page):
    rows = []
    for invoice in page:
        if not isinstance(invoice, dict):
            continue
        metadata = _invoice_metadata(invoice)
        rows.append(
            {
                "id": _invoice_stable_id(invoice),
                "status": invoice.get("status"),
                "orderId": invoice.get("orderId") or metadata.get("orderId"),
                "orderUrl": invoice.get("orderUrl") or metadata.get("orderUrl"),
                "paymentRequestId": invoice.get("paymentRequestId")
                or metadata.get("paymentRequestId")
                or metadata.get("payment_request_id"),
                "metadata": metadata,
                "payments": invoice.get("payments"),
            }
        )
    return sorted(rows, key=lambda row: row["id"])


def _invoice_page_fingerprint(page):
    return hashlib.sha256(
        json.dumps(_invoice_page_fingerprint_rows(page), sort_keys=True).encode("utf-8")
    ).hexdigest()


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
        if exc.code == 429:
            raise AppError(
                f"BTCPay rate limited the request (HTTP 429) for {url}",
                code="rate_limited",
                retryable=True,
                details={"retry_after_seconds": retry_after_seconds_from_http_error(exc)},
            ) from exc
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
    normalized = str(payment_method_id or "").strip().upper()
    return normalized in WALLET_HISTORY_PAYMENT_METHOD_IDS


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


def _normalize_invoice_provenance(store_id, invoice):
    if not isinstance(invoice, dict):
        raise AppError("BTCPay invoice record was not a JSON object", code="protocol_error")
    invoice_id = invoice.get("id") or invoice.get("invoiceId")
    if not invoice_id:
        raise AppError("BTCPay invoice record is missing 'id'", code="protocol_error")
    payments = invoice.get("payments") or []
    if not isinstance(payments, list):
        payments = []
    metadata = _invoice_metadata(invoice)
    order_id = _str_or_none(invoice.get("orderId") or metadata.get("orderId"))
    origin = _invoice_origin(invoice, metadata, order_id)
    return {
        "store_id": store_id,
        "invoice": invoice,
        "invoice_id": str(invoice_id),
        "order_id": order_id,
        "order_url": _str_or_none(metadata.get("orderUrl") or invoice.get("orderUrl")),
        "payment_request_id": _str_or_none(
            metadata.get("paymentRequestId")
            or metadata.get("payment_request_id")
            or invoice.get("paymentRequestId")
        ),
        "origin_kind": origin["kind"],
        "origin_app_id": origin["app_id"],
        "origin_label": origin["label"],
        "origin_url": origin["url"],
        "status": _str_or_none(invoice.get("status")),
        "created_at": _btcpay_time(invoice.get("createdTime") or invoice.get("created")),
        "currency": _str_or_none(invoice.get("currency")),
        "amount": _str_or_none(invoice.get("amount")),
        "payments": [_normalize_invoice_payment(invoice, payment) for payment in payments if isinstance(payment, dict)],
    }


def _normalize_invoice_payment(invoice, payment):
    details = payment.get("details") if isinstance(payment.get("details"), dict) else {}
    method = (
        payment.get("paymentMethod")
        or payment.get("paymentMethodId")
        or payment.get("paymentMethodData")
        or details.get("paymentMethod")
    )
    return {
        "payment": payment,
        "payment_id": _str_or_none(
            payment.get("id")
            or payment.get("paymentId")
            or payment.get("accountedPaymentId")
            or payment.get("transactionId")
            or details.get("transactionId")
        ),
        "payment_method_id": _str_or_none(method),
        "status": _str_or_none(payment.get("status") or details.get("status")),
        "received_at": _btcpay_time(
            payment.get("receivedDate")
            or payment.get("receivedTime")
            or payment.get("createdTime")
            or details.get("receivedDate")
        ),
        "amount": _str_or_none(
            payment.get("value")
            or payment.get("cryptoAmount")
            or payment.get("amount")
            or details.get("value")
        ),
        "rate": _str_or_none(payment.get("rate") or details.get("rate")),
        "txid": _str_or_none(
            payment.get("transactionId")
            or payment.get("transactionHash")
            or details.get("transactionId")
            or details.get("transactionHash")
        ),
        "payment_hash": _str_or_none(
            payment.get("paymentHash")
            or payment.get("preimageHash")
            or details.get("paymentHash")
            or details.get("preimageHash")
        ),
        "destination": _str_or_none(
            payment.get("destination")
            or payment.get("address")
            or details.get("destination")
            or details.get("address")
        ),
        "invoice_currency": _str_or_none(invoice.get("currency")),
        "invoice_amount": _str_or_none(invoice.get("amount")),
    }


def _invoice_metadata(invoice):
    metadata = invoice.get("metadata")
    if isinstance(metadata, dict):
        return metadata
    return {}


def _invoice_origin(invoice, metadata, order_id):
    order_url = _str_or_none(metadata.get("orderUrl") or invoice.get("orderUrl"))
    app_id = _str_or_none(
        metadata.get("appId")
        or metadata.get("app_id")
        or metadata.get("applicationId")
    )
    app_name = _str_or_none(
        metadata.get("appName")
        or metadata.get("app_name")
        or metadata.get("applicationName")
    )
    item_desc = _str_or_none(metadata.get("itemDesc") or metadata.get("itemDescription"))
    pos_data = metadata.get("posData")
    pos_label = _pos_data_label(pos_data)
    payment_request_id = _str_or_none(
        metadata.get("paymentRequestId")
        or metadata.get("payment_request_id")
        or invoice.get("paymentRequestId")
    )

    lower_order_url = (order_url or "").lower()
    lower_order_id = (order_id or "").lower()
    if pos_data is not None or "/pos" in lower_order_url or lower_order_id.startswith("pos"):
        return {
            "kind": "pos",
            "app_id": app_id,
            "label": app_name or item_desc or pos_label or order_id,
            "url": order_url,
        }
    if app_id or app_name:
        return {
            "kind": "app",
            "app_id": app_id,
            "label": app_name or item_desc or order_id,
            "url": order_url,
        }
    if order_url or order_id:
        return {
            "kind": "external_order",
            "app_id": None,
            "label": item_desc or order_id,
            "url": order_url,
        }
    if payment_request_id:
        return {
            "kind": "payment_request",
            "app_id": None,
            "label": item_desc or payment_request_id,
            "url": None,
        }
    return {"kind": "unknown", "app_id": None, "label": item_desc, "url": order_url}


def _pos_data_label(pos_data):
    if not isinstance(pos_data, dict):
        return None
    for key in ("title", "name", "itemDesc", "itemDescription", "description"):
        value = _str_or_none(pos_data.get(key))
        if value:
            return value
    return None


def _str_or_none(value):
    if value in (None, ""):
        return None
    return str(value)


def _btcpay_time(value):
    if value in (None, ""):
        return None
    if isinstance(value, str) and any(char in value for char in ("T", "Z", "+")):
        return value
    return _unix_to_iso(value)


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
    "WALLET_HISTORY_PAYMENT_METHOD_IDS",
    "discover_btcpay_wallet_sources",
    "fetch_btcpay_invoice_provenance",
    "fetch_btcpay_records",
    "probe_btcpay_wallet",
    "require_wallet_history_payment_method",
]
