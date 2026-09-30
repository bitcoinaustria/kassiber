"""BTCPay Greenfield fetchers used by the CLI, daemon, and freshness jobs.

Every function here performs user-triggered, read-only ``GET`` requests
through ``kassiber.btcpay.client.GreenfieldClient``:

- ``fetch_btcpay_records``: confirmed on-chain wallet history
  (``/payment-methods/{id}/wallet/transactions``), normalised into the same
  shape as the BTCPay CSV importer so comments become notes and labels
  become tags. BTCPay gates this endpoint behind the store *modify*
  permission.
- ``fetch_btcpay_invoice_provenance``: invoices with their payments, for
  commercial provenance. Needs only read permissions.
- ``fetch_btcpay_payouts``: refunds, pull-payment claims, and store payouts.
- ``inspect_btcpay_backend`` / ``discover_btcpay_wallet_sources``: setup
  inspection (server, key permissions, stores, payment methods, wallets).
- ``probe_btcpay_instance`` / ``probe_btcpay_wallet``: one-request checks.

Wallet history and invoices are paged with an incremental checkpoint: a page
whose stable ids and fingerprint match the previous run is skipped, the walk
stops after a window of unchanged pages, and one older page is re-audited per
run so edits deep in history are still picked up.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
from typing import Any, Mapping
from urllib import parse as urlparse

from .backends import backend_value
from .btcpay.client import GreenfieldClient, GreenfieldHttpError, greenfield_app_error
from .btcpay.discovery import InspectionResult, inspect_btcpay_connection
from .btcpay.origins import classify_invoice_origin, payment_request_id_for
from .btcpay.payment_methods import (
    WALLET_HISTORY_PAYMENT_METHOD_IDS,
    canonical_payment_method_id,
    classify_payment_method,
    is_wallet_history_payment_method,
    require_wallet_history_payment_method,
    split_payment_method_id,
)
from .btcpay.payouts import normalize_payout
from .core.sync import emit_sync_progress
from .errors import AppError
from .importers import normalize_btcpay_record, parse_btcpay_labels
from .proxy import build_proxy_opener


DEFAULT_PAYMENT_METHOD_ID = "BTC-CHAIN"
DEFAULT_PAGE_SIZE = 100
DEFAULT_STATUS_FILTER = "Confirmed"
MAX_PAGES = 10_000
INCREMENTAL_UNCHANGED_PAGE_WINDOW = 5
INCREMENTAL_DEEP_AUDIT_PAGES = 1

WALLET_HISTORY_PERMISSION_HINT = (
    "Greenfield wallet endpoints currently require the "
    "`btcpay.store.canmodifystoresettings` permission. Keep this store as "
    "invoice provenance with a read-only key, or map it to a watch-only wallet "
    "you already track."
)
INVOICE_PERMISSION_HINT = "Grant the API key the BTCPay 'View invoices' permission."
STORE_PERMISSION_HINT = "Grant the API key 'View your stores' (btcpay.store.canviewstoresettings)."

# Invoice states whose payment list can still change or that may carry
# payments. Expired/New invoices without an additional status were never paid.
_INVOICE_STATES_WITH_PAYMENTS = {"settled", "processing", "invalid", "paid", "confirmed", "complete"}
_INVOICE_STATES_OPEN = {"new", "processing", "paid"}
_UNPAID_ADDITIONAL_STATUSES = {"", "none"}


def _backend_http_opener(backend):
    return build_proxy_opener(
        backend_value(backend, "tor_proxy", "proxy"),
        source_label="BTCPay",
    )


def _client(backend, opener=None, *, missing_token_hint=None) -> GreenfieldClient:
    return GreenfieldClient.from_backend(
        backend,
        opener=opener,
        opener_factory=_backend_http_opener,
        missing_token_hint=missing_token_hint,
    )


def _require_store(store_id):
    if not store_id:
        raise AppError("BTCPay store id is required", code="validation")


def _require_page_size(page_size):
    if page_size <= 0:
        raise AppError("BTCPay page_size must be positive", code="validation")


def _json_array(payload, url_label):
    if not isinstance(payload, list):
        raise AppError(
            f"BTCPay response for {url_label} was not a JSON array",
            code="protocol_error",
        )
    return payload


def _wallet_transactions_path(store_id, payment_method_id, legacy=False):
    store_q = urlparse.quote(store_id, safe="")
    if legacy:
        currency, _ = split_payment_method_id(payment_method_id)
        code_q = urlparse.quote(currency, safe="")
        return f"/api/v1/stores/{store_q}/payment-methods/onchain/{code_q}/wallet/transactions"
    payment_q = urlparse.quote(payment_method_id, safe="")
    return f"/api/v1/stores/{store_q}/payment-methods/{payment_q}/wallet/transactions"


def _wallet_page_fetcher(client, store_id, payment_method_id, page_size, api_state):
    def fetch_page(skip, limit=None):
        query = {
            "statusFilter": DEFAULT_STATUS_FILTER,
            "skip": str(skip),
            "limit": str(limit or page_size),
        }
        legacy = bool(api_state.get("legacy_wallet_paths"))
        try:
            page = client.get_url(client.url(_wallet_transactions_path(store_id, payment_method_id, legacy), query))
        except GreenfieldHttpError as exc:
            if exc.status == 404 and not legacy and not exc.error_code:
                # BTCPay 1.x keeps wallet endpoints under /payment-methods/onchain/{code}.
                try:
                    page = client.get_url(
                        client.url(_wallet_transactions_path(store_id, payment_method_id, True), query)
                    )
                except GreenfieldHttpError as legacy_exc:
                    raise greenfield_app_error(
                        exc if legacy_exc.status == 404 else legacy_exc,
                        context="wallet history",
                        permission_hint=WALLET_HISTORY_PERMISSION_HINT,
                    ) from legacy_exc
                api_state["legacy_wallet_paths"] = True
            else:
                raise greenfield_app_error(
                    exc,
                    context="wallet history",
                    permission_hint=WALLET_HISTORY_PERMISSION_HINT,
                ) from exc
        return _json_array(page, "wallet transactions")

    return fetch_page


def fetch_btcpay_records(
    backend,
    store_id,
    payment_method_id=DEFAULT_PAYMENT_METHOD_ID,
    page_size=DEFAULT_PAGE_SIZE,
    opener=None,
    checkpoint=None,
    metadata=None,
):
    _require_store(store_id)
    payment_method_id = require_wallet_history_payment_method(payment_method_id)
    client = _client(backend, opener)
    _require_page_size(page_size)
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    previous_pages = checkpoint.get("btcpay_pages") or {}
    previous_pagination = checkpoint.get("btcpay_pagination") or {}
    api_state = {"legacy_wallet_paths": bool(previous_pagination.get("legacy_wallet_paths"))}
    fetch_page = _wallet_page_fetcher(client, store_id, payment_method_id, page_size, api_state)

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
    pagination = dict(page_metadata["pagination"])
    if api_state.get("legacy_wallet_paths"):
        pagination["legacy_wallet_paths"] = True
    if metadata is not None:
        metadata.update(
            {
                "btcpay_pages": _sorted_page_map(next_pages),
                "btcpay_pagination": pagination,
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
    """Fetch invoice/payment provenance without importing wallet balances.

    Invoice pages are fingerprinted as listed. Only pages that changed are
    hydrated, and only invoices that can carry payments trigger the
    per-invoice payment request older servers need when the list omits
    ``paymentMethods``. Open invoices (New/Processing) always count as changed
    so a second partial payment is not missed.
    """

    _require_store(store_id)
    client = _client(backend, opener)
    _require_page_size(page_size)
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    previous_pages = checkpoint.get("btcpay_invoice_pages") or {}
    previous_pagination = checkpoint.get("btcpay_invoice_pagination") or {}
    store_q = urlparse.quote(store_id, safe="")
    lookup_state: dict[str, Any] = {"payment_request_titles": None, "hydrated": 0}

    def fetch_page(skip):
        page = client.get_json(
            f"/api/v1/stores/{store_q}/invoices",
            {"skip": str(skip), "take": str(page_size), "includePaymentMethods": "true"},
            context="invoices",
            permission_hint=INVOICE_PERMISSION_HINT,
        )
        return _json_array(page, "invoices")

    def normalize_page(page):
        hydrated = _hydrate_invoice_payment_methods(client, store_id, page, lookup_state)
        normalized = [_normalize_invoice_provenance(store_id, invoice) for invoice in hydrated]
        unlabeled = [
            index
            for index, invoice in enumerate(normalized)
            if invoice["origin_kind"] == "payment_request"
            and invoice["origin_label"] == invoice["payment_request_id"]
        ]
        if unlabeled:
            titles = _payment_request_titles(client, store_id, lookup_state)
            for index in unlabeled:
                title = titles.get(normalized[index]["payment_request_id"])
                if title:
                    normalized[index]["origin_label"] = title
        return normalized

    invoices, next_pages, page_metadata = _fetch_incremental_pages(
        fetch_page=fetch_page,
        page_size=page_size,
        previous_pages=previous_pages,
        previous_pagination=previous_pagination,
        fingerprint_fn=_invoice_page_fingerprint,
        stable_ids_fn=_invoice_page_stable_ids,
        normalize_page=normalize_page,
        max_pages_message=f"BTCPay invoice sync exceeded {MAX_PAGES} pages; aborting for safety",
        force_changed_fn=_invoice_page_has_open_invoice,
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
                "invoices_hydrated": lookup_state["hydrated"],
            }
        )
    return invoices


def fetch_btcpay_payouts(backend, store_id, *, opener=None, metadata=None):
    """Fetch refunds, pull-payment claims, and store payouts for one store.

    Cancelled payouts are excluded. A key without the payouts permission is
    not an error: invoice provenance still works, and ``metadata`` reports
    ``payouts_permission_missing`` so the caller can explain the gap.
    """

    _require_store(store_id)
    client = _client(backend, opener)
    store_q = urlparse.quote(store_id, safe="")
    payouts_payload, payouts_error = client.get_optional_json(
        f"/api/v1/stores/{store_q}/payouts",
        {"includeCancelled": "false"},
        tolerate=(403, 404),
    )
    if payouts_error is not None:
        if metadata is not None:
            metadata.update(
                {
                    "payouts_available": False,
                    "payouts_permission_missing": payouts_error.status == 403,
                    "payouts_seen": 0,
                }
            )
        return []
    payouts = _json_array(payouts_payload, "payouts")
    pull_payments: dict[str, Mapping[str, Any]] = {}
    if any(isinstance(payout, Mapping) and payout.get("pullPaymentId") for payout in payouts):
        pull_payload, pull_error = client.get_optional_json(
            f"/api/v1/stores/{store_q}/pull-payments",
            {"includeArchived": "true"},
            tolerate=(403, 404),
        )
        if pull_error is None and isinstance(pull_payload, list):
            pull_payments = {
                str(item.get("id")): item
                for item in pull_payload
                if isinstance(item, Mapping) and item.get("id")
            }
    normalized = []
    for payout in payouts:
        if not isinstance(payout, Mapping):
            raise AppError("BTCPay payout record was not a JSON object", code="protocol_error")
        record = normalize_payout(store_id, payout, pull_payments=pull_payments)
        if record is not None:
            normalized.append(record)
    if metadata is not None:
        metadata.update(
            {
                "payouts_available": True,
                "payouts_permission_missing": False,
                "payouts_seen": len(normalized),
            }
        )
    return normalized


def probe_btcpay_wallet(
    backend,
    store_id,
    payment_method_id=DEFAULT_PAYMENT_METHOD_ID,
    opener=None,
):
    """Validate one BTCPay wallet-history request without walking the paginator."""

    _require_store(store_id)
    payment_method_id = require_wallet_history_payment_method(payment_method_id)
    client = _client(backend, opener)
    fetch_page = _wallet_page_fetcher(client, store_id, payment_method_id, 1, {})
    page = fetch_page(0, 1)
    return {"checked": True, "rows_seen": len(page)}


def probe_btcpay_instance(backend, opener=None):
    """Validate BTCPay reachability and API-key store access.

    Connection health checks do not know which store/payment-method route a
    backend will eventually serve. Keep that probe to a single store-catalog
    request; setup inspection is the explicit, richer path.
    """

    client = _client(
        backend,
        opener,
        missing_token_hint="Enter a Greenfield API key for this BTCPay instance.",
    )
    stores = client.get_json(
        "/api/v1/stores",
        context="the store list",
        permission_hint="Grant the API key access to view stores.",
    )
    _json_array(stores, "stores")
    return {"checked": True, "stores_seen": len(stores)}


def inspect_btcpay_backend(backend, opener=None, **kwargs) -> InspectionResult:
    """Full setup inspection. See ``kassiber.btcpay.discovery``."""

    client = _client(
        backend,
        opener,
        missing_token_hint="Enter a Greenfield API key for this BTCPay instance.",
    )
    return inspect_btcpay_connection(client, **kwargs)


def discover_btcpay_wallet_sources(backend, opener=None):
    """Return the public inspection payload (stores, payment methods, key, server).

    The address previews used for wallet recognition are dropped; call
    ``inspect_btcpay_backend`` when local ownership matching needs them.
    """

    return inspect_btcpay_backend(backend, opener=opener).public


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
    force_changed_fn=None,
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
    terminal_skip: int | None = None
    unchanged_streak = 0
    skip = 0

    def load_page(page_skip):
        nonlocal pages_fetched
        if pages_fetched >= MAX_PAGES:
            raise AppError(max_pages_message, code="config_error")
        emit_sync_progress({"phase": "backend_fetch"})
        page = fetch_page(page_skip)
        pages_fetched += 1
        fetched_skips.add(page_skip)
        emit_sync_progress(
            {
                "phase": "backend_fetch",
                "pages_fetched": pages_fetched,
                "page_size": page_size,
            }
        )
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
        ) and not (force_changed_fn is not None and force_changed_fn(page))
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
                "additionalStatus": invoice.get("additionalStatus"),
                "paidAmount": invoice.get("paidAmount"),
                "orderId": invoice.get("orderId") or metadata.get("orderId"),
                "orderUrl": invoice.get("orderUrl") or metadata.get("orderUrl"),
                "paymentRequestId": invoice.get("paymentRequestId")
                or metadata.get("paymentRequestId")
                or metadata.get("payment_request_id"),
                "metadata": metadata,
                "paymentMethods": invoice.get("paymentMethods"),
                "payments": invoice.get("payments"),
            }
        )
    return sorted(rows, key=lambda row: row["id"])


def _invoice_page_fingerprint(page):
    return hashlib.sha256(
        json.dumps(_invoice_page_fingerprint_rows(page), sort_keys=True).encode("utf-8")
    ).hexdigest()


def _invoice_status(invoice):
    return str(invoice.get("status") or "").strip().lower()


def _invoice_additional_status(invoice):
    return str(invoice.get("additionalStatus") or "").strip().lower()


def _invoice_may_have_payments(invoice):
    if _invoice_status(invoice) in _INVOICE_STATES_WITH_PAYMENTS:
        return True
    return _invoice_additional_status(invoice) not in _UNPAID_ADDITIONAL_STATUSES


def _invoice_is_open(invoice):
    """An invoice that is still collecting or confirming payments."""

    if not isinstance(invoice, dict):
        return False
    status = _invoice_status(invoice)
    if status == "processing":
        return True
    return status in _INVOICE_STATES_OPEN and _invoice_additional_status(invoice) not in _UNPAID_ADDITIONAL_STATUSES


def _invoice_page_has_open_invoice(page):
    return any(_invoice_is_open(invoice) for invoice in page)


def _hydrate_invoice_payment_methods(client, store_id, invoices, lookup_state):
    hydrated = []
    store_q = urlparse.quote(store_id, safe="")
    for invoice in invoices:
        if not isinstance(invoice, dict):
            hydrated.append(invoice)
            continue
        methods = invoice.get("paymentMethods")
        if isinstance(methods, list):
            copy = dict(invoice)
            copy["payments"] = _payments_from_invoice_payment_methods(methods)
            hydrated.append(copy)
            continue
        payments = invoice.get("payments")
        if isinstance(payments, list) and payments:
            hydrated.append(invoice)
            continue
        invoice_id = invoice.get("id") or invoice.get("invoiceId")
        if not invoice_id or not _invoice_may_have_payments(invoice):
            hydrated.append(invoice)
            continue
        invoice_q = urlparse.quote(str(invoice_id), safe="")
        methods = client.get_json(
            f"/api/v1/stores/{store_q}/invoices/{invoice_q}/payment-methods",
            {"onlyAccountedPayments": "true"},
            context="invoice payments",
            permission_hint=INVOICE_PERMISSION_HINT,
        )
        methods = _json_array(methods, "invoice payment methods")
        lookup_state["hydrated"] = int(lookup_state.get("hydrated") or 0) + 1
        copy = dict(invoice)
        copy["paymentMethods"] = methods
        copy["payments"] = _payments_from_invoice_payment_methods(methods)
        hydrated.append(copy)
    return hydrated


def _payment_request_titles(client, store_id, lookup_state):
    """Load payment-request titles once per sync, tolerating a key without access."""

    if lookup_state.get("payment_request_titles") is not None:
        return lookup_state["payment_request_titles"]
    store_q = urlparse.quote(store_id, safe="")
    payload, error = client.get_optional_json(
        f"/api/v1/stores/{store_q}/payment-requests",
        tolerate=(403, 404),
    )
    titles = {}
    if error is None and isinstance(payload, list):
        for item in payload:
            if isinstance(item, Mapping) and item.get("id") and item.get("title"):
                titles[str(item["id"])] = str(item["title"])
    lookup_state["payment_request_titles"] = titles
    return titles


def _payments_from_invoice_payment_methods(methods):
    payments = []
    for method in methods:
        if not isinstance(method, dict):
            continue
        payment_method_id = canonical_payment_method_id(
            method.get("paymentMethodId") or method.get("paymentMethod")
        ) or None
        method_payments = method.get("payments") or []
        if not isinstance(method_payments, list):
            continue
        for payment in method_payments:
            if not isinstance(payment, dict):
                continue
            enriched = dict(payment)
            enriched.setdefault("paymentMethodId", payment_method_id)
            if method.get("rate") is not None:
                enriched.setdefault("rate", method.get("rate"))
            if method.get("destination") is not None:
                enriched.setdefault("destination", method.get("destination"))
            payment_id = str(enriched.get("id") or "")
            txid = _txid_from_payment_id(payment_id, payment_method_id)
            if not enriched.get("transactionId") and txid:
                enriched["transactionId"] = txid
            payments.append(enriched)
    return payments


def _looks_like_txid(value):
    return len(value) == 64 and all(char in "0123456789abcdefABCDEF" for char in value)


def _is_chain_payment_method(payment_method_id):
    return canonical_payment_method_id(payment_method_id).endswith("-CHAIN")


def _txid_from_payment_id(value, payment_method_id=None):
    raw = str(value or "").strip()
    if _looks_like_txid(raw) and _is_chain_payment_method(payment_method_id):
        return raw
    first, separator, _ = raw.partition("-")
    if separator and _looks_like_txid(first):
        return first
    return None


def _payment_hash_from_payment_id(value, payment_method_id=None):
    # BTCPay identifies a Lightning payment by its payment hash.
    raw = str(value or "").strip().lower()
    if _looks_like_txid(raw) and classify_payment_method(payment_method_id)["rail"] in {"lightning", "lnurl"}:
        return raw
    return None


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


def _normalize_invoice_provenance(store_id, invoice, *, payment_request_titles=None):
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
    origin = classify_invoice_origin(
        invoice,
        metadata,
        order_id,
        payment_request_titles=payment_request_titles,
    )
    return {
        "store_id": store_id,
        "invoice": invoice,
        "invoice_id": str(invoice_id),
        "order_id": order_id,
        "order_url": _str_or_none(metadata.get("orderUrl") or invoice.get("orderUrl")),
        "payment_request_id": payment_request_id_for(invoice, metadata, order_id),
        "origin_kind": origin["kind"],
        "origin_app_id": origin["app_id"],
        "origin_app_type": origin["app_type"],
        "origin_source": origin["source"],
        "origin_label": origin["label"],
        "origin_url": origin["url"],
        "status": _str_or_none(invoice.get("status")),
        "additional_status": _str_or_none(invoice.get("additionalStatus")),
        "invoice_type": _str_or_none(invoice.get("type")),
        "created_at": _btcpay_time(invoice.get("createdTime") or invoice.get("created")),
        "currency": _str_or_none(invoice.get("currency")),
        "amount": _str_or_none(invoice.get("amount")),
        "payments": [_normalize_invoice_payment(invoice, payment) for payment in payments if isinstance(payment, dict)],
    }


def _normalize_invoice_payment(invoice, payment):
    details = payment.get("details") if isinstance(payment.get("details"), dict) else {}
    method = canonical_payment_method_id(
        payment.get("paymentMethod")
        or payment.get("paymentMethodId")
        or payment.get("paymentMethodData")
        or details.get("paymentMethod")
    ) or None
    payment_id = (
        payment.get("id")
        or payment.get("paymentId")
        or payment.get("accountedPaymentId")
        or payment.get("transactionId")
        or details.get("transactionId")
    )
    return {
        "payment": payment,
        "payment_id": _str_or_none(payment_id),
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
        "fee": _str_or_none(payment.get("fee") or details.get("fee")),
        "rate": _str_or_none(payment.get("rate") or details.get("rate")),
        "txid": _str_or_none(
            payment.get("transactionId")
            or payment.get("transactionHash")
            or details.get("transactionId")
            or details.get("transactionHash")
            or _txid_from_payment_id(payment_id, method)
        ),
        "payment_hash": _str_or_none(
            payment.get("paymentHash")
            or payment.get("preimageHash")
            or details.get("paymentHash")
            or details.get("preimageHash")
            or _payment_hash_from_payment_id(payment_id, method)
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
    "fetch_btcpay_payouts",
    "fetch_btcpay_records",
    "inspect_btcpay_backend",
    "is_wallet_history_payment_method",
    "probe_btcpay_instance",
    "probe_btcpay_wallet",
    "require_wallet_history_payment_method",
]
