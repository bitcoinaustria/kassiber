"""BTCPay Greenfield API fetcher for on-chain wallet transactions.

The public entry point is `fetch_btcpay_records(backend, store_id, ...)`,
which hits `GET /api/v1/stores/{storeId}/payment-methods/{paymentMethodId}/wallet/transactions`,
pages through the result with `skip`/`limit`, and returns records in the
same shape `kassiber.importers.normalize_btcpay_record` already produces.
That means the coordinator in `app.py` can feed the output straight into
`insert_wallet_records` + `apply_btcpay_metadata` the same way as
CSV/JSON BTCPay imports — no second code path for notes and labels.

Auth: `Authorization: token <api-key>` header (not `Bearer`). Greenfield
wallet endpoints currently require the `btcpay.store.canmodifystoresettings`
scope because the same path also serves tx creation/broadcast; there is no
read-only wallet-view scope upstream yet.

Pagination: the Greenfield list endpoint has no date-range filter — only
`skip`/`limit`. We page until a response is smaller than `page_size`;
`insert_wallet_records` deduplicates on re-runs via the fingerprint column,
so sync is safe to repeat.
"""

import datetime as _dt
import json
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .backends import backend_timeout, backend_value
from .errors import AppError
from .importers import normalize_btcpay_record


DEFAULT_PAYMENT_METHOD_ID = "BTC-CHAIN"
DEFAULT_PAGE_SIZE = 100
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
    base = backend_value(backend, "url")
    if not base:
        raise AppError("BTCPay backend is missing 'url'", code="config_error")
    token = backend_value(backend, "token")
    if not token:
        raise AppError(
            "BTCPay backend is missing 'token' (api key)",
            code="config_error",
            hint="Store the api key with `kassiber backends update --token <key>` or KASSIBER_BACKEND_<NAME>_TOKEN.",
        )
    if page_size <= 0:
        raise AppError("BTCPay page_size must be positive", code="validation")
    timeout = backend_timeout(backend)
    http_opener = opener or urlrequest.build_opener()
    raw_records = []
    skip = 0
    page_count = 0
    while True:
        if page_count >= MAX_PAGES:
            raise AppError(
                f"BTCPay sync exceeded {MAX_PAGES} pages; aborting for safety",
                code="config_error",
            )
        url = _build_list_url(base, store_id, payment_method_id, skip, page_size)
        page = _http_get_json(http_opener, url, token, timeout)
        if not isinstance(page, list):
            raise AppError(
                f"BTCPay response for {url} was not a JSON array",
                code="protocol_error",
            )
        raw_records.extend(page)
        page_count += 1
        if len(page) < page_size:
            break
        skip += page_size
    return [_to_record(tx, payment_method_id) for tx in raw_records]


def _build_list_url(base, store_id, payment_method_id, skip, limit):
    base = base.rstrip("/")
    store_q = urlparse.quote(store_id, safe="")
    pm_q = urlparse.quote(payment_method_id, safe="")
    query = urlparse.urlencode({"skip": str(skip), "limit": str(limit)})
    return f"{base}/api/v1/stores/{store_q}/payment-methods/{pm_q}/wallet/transactions?{query}"


def _http_get_json(opener, url, token, timeout):
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
                hint="Greenfield wallet endpoints require the `btcpay.store.canmodifystoresettings` scope.",
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


def _to_record(tx, payment_method_id):
    """Shape a Greenfield `OnChainWalletTransactionData` into the dict
    `normalize_btcpay_record` already knows how to parse, then return
    the normalized import record.

    The asset is derived from the `paymentMethodId` prefix (e.g.
    `BTC-CHAIN` -> `BTC`). The `labels` array is `{type, text}` objects
    in the Greenfield schema — we keep the `text` values since the CSV
    export flattens to names, and `parse_btcpay_labels` already accepts
    a list.
    """
    if not isinstance(tx, dict):
        raise AppError("BTCPay transaction record was not a JSON object", code="protocol_error")
    currency = payment_method_id.split("-", 1)[0].upper() if payment_method_id else "BTC"
    timestamp = tx.get("timestamp")
    if timestamp is None:
        raise AppError("BTCPay transaction is missing 'timestamp'", code="protocol_error")
    occurred_at = _unix_to_iso(timestamp)
    labels_raw = tx.get("labels")
    label_names = []
    if isinstance(labels_raw, list):
        for item in labels_raw:
            if isinstance(item, dict):
                text = item.get("text")
            else:
                text = item
            if text:
                label_names.append(str(text))
    csv_shaped = {
        "TransactionId": tx.get("transactionHash") or "",
        "Timestamp": occurred_at,
        "Currency": currency,
        "Amount": str(tx.get("amount") if tx.get("amount") is not None else "0"),
        "Comment": tx.get("comment") or "",
        "Labels": label_names,
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
    return (
        _dt.datetime.fromtimestamp(value, tz=_dt.timezone.utc)
        .isoformat()
        .replace("+00:00", "Z")
    )
