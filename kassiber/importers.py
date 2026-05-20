"""File-format parsers for wallet-export imports.

Call sites hand a file path + format tag in and get back a list of
normalized record dicts out — one dict per transaction, with the same
shape across all formats so downstream DB-insertion code
(`insert_wallet_records` in `kassiber.core.imports`) doesn't need to
branch per wallet.

Import format families live here:

  - Generic JSON / CSV — user-supplied, pass-through (no asset-specific
    parsing beyond CSV-row decoding).
  - BTCPay Server (`btcpay_json`, `btcpay_csv`) — on-chain receive/send
    history exported from a BTCPay store. `normalize_btcpay_record`
    pulls the `_btcpay_comment` and `_btcpay_labels` fields so
    `apply_btcpay_metadata` can later set transaction notes and tags.
  - Phoenix (`phoenix_csv`) — Lightning-native CSV from the Phoenix
    mobile wallet. Amounts are stored as signed INTEGER msat, fees are
    split across `mining_fee_sat` + `service_fee_msat`, and
    `amount_fiat` is "value CCY" formatted. We normalize all of it to
    the common record shape (BTC Decimals + derived `fiat_rate`).
  - River (`river_csv`) — Bitcoin Activity or Account Activity CSV
    exports. BTC-side rows are normalized to Kassiber transactions and
    exact fiat execution amounts are preserved as exchange pricing
    provenance where the export provides the paired cash leg.
  - Bull Bitcoin (`bullbitcoin_csv`) — order export CSV. Completed
    Bitcoin on-chain, Lightning, or Liquid <-> fiat orders are normalized
    to exchange execution pricing, keyed by the exported transaction id
    when present.
  - Coinfinity (`coinfinity_csv`) — order export CSV. BTC/EUR broker rows
    are normalized as exact exchange execution evidence, with fiat fees
    folded into the taxable buy/sell value.
  - 21bitcoin (`21bitcoin_csv`) — transaction CSV export. BTC-side trades
    and withdrawals are normalized as a custodial platform ledger; fiat-only
    cash rows are skipped.
  - Pocket Bitcoin (`pocketbitcoin_csv`) — account CSV export. Exchange rows
    are normalized as exact execution evidence and paired with adjacent BTC
    withdrawal rows when the export records the on-chain payout separately.
  - Strike (`strike_csv`) — custodial platform CSV export. Bitcoin Lightning,
    on-chain, and BTC exchange rows are normalized as active platform ledger
    rows; fiat-only funding rows are skipped.
  - BIP329 JSONL — one-record-per-line label export. `record_type`
    distinguishes tx/addr/pubkey/input/output/xpub labels.

`load_import_records(path, input_format)` is the dispatcher used by the
coordinator layer. Adding a new format: add a `normalize_<fmt>_record`
and a `load_<fmt>_records`, plug them into `load_import_records`, and
add an `is_<fmt>_format` predicate so the metadata-application layer in
`kassiber.core.imports` can hook format-specific side effects.

Every parser raises `AppError` on unparseable input so the CLI surfaces
a validation envelope rather than a bare `ValueError` / `KeyError`.
"""

import csv
import json
import os
from datetime import datetime, timezone
from decimal import Decimal

from .envelope import json_ready
from .errors import AppError
from .msat import dec, msat_to_btc
from .util import str_or_none
from .wallet_descriptors import normalize_asset_code


# -- generic coordinator -----------------------------------------------------


def load_import_records(file_path, input_format):
    """Dispatch on `input_format` to the matching loader.

    Returns a list of dicts with a format-appropriate shape. Generic
    JSON / CSV pass through raw rows; BTCPay and Phoenix loaders
    return already-normalized records ready for
    `normalize_import_record`.
    """
    if not os.path.exists(file_path):
        raise AppError(
            f"Import file not found: {file_path}",
            code="not_found",
            hint="Check the wallet source_file or pass an existing --file path.",
        )
    if input_format == "json":
        with open(file_path, "r", encoding="utf-8") as handle:
            payload = json.load(handle)
        if isinstance(payload, dict):
            payload = payload.get("transactions", [])
        if not isinstance(payload, list):
            raise AppError("JSON import must be a list of transaction objects")
        return payload
    if input_format == "csv":
        with open(file_path, "r", encoding="utf-8", newline="") as handle:
            return list(csv.DictReader(handle))
    if input_format == "btcpay_json":
        return load_btcpay_export_records(file_path, "json")
    if input_format == "btcpay_csv":
        return load_btcpay_export_records(file_path, "csv")
    if input_format == "phoenix_csv":
        return load_phoenix_csv_records(file_path)
    if input_format == "river_csv":
        return load_river_csv_records(file_path)
    if input_format == "bullbitcoin_csv":
        return load_bullbitcoin_csv_records(file_path)
    if input_format == "coinfinity_csv":
        return load_coinfinity_csv_records(file_path)
    if input_format == "21bitcoin_csv":
        return load_twentyonebitcoin_csv_records(file_path)
    if input_format == "pocketbitcoin_csv":
        return load_pocketbitcoin_csv_records(file_path)
    if input_format == "strike_csv":
        return load_strike_csv_records(file_path)
    raise AppError(f"Unsupported input format '{input_format}'")


# -- BTCPay ------------------------------------------------------------------


def parse_btcpay_amount(amount_text, currency=None):
    """Parse a BTCPay amount cell like `"0.001 BTC"` into a signed `Decimal`."""
    if amount_text is None:
        raise AppError("BTCPay export is missing Amount")
    text = str(amount_text).strip()
    asset = str(currency or "BTC").strip().upper()
    suffixes = [asset, asset.lower(), asset.upper()]
    for suffix in suffixes:
        if suffix and text.endswith(suffix):
            text = text[: -len(suffix)].strip()
            break
    return dec(text)


def parse_btcpay_labels(value):
    """Normalize BTCPay labels from CSV, JSON export, or Greenfield shapes."""
    if value is None or value == "":
        return []
    labels = []
    if isinstance(value, dict):
        for key, item in value.items():
            text = item.get("text") if isinstance(item, dict) else item
            if text is None and key:
                text = key
            if text is None:
                continue
            label = str(text).strip()
            if label:
                labels.append(label)
        return labels
    if isinstance(value, list):
        for item in value:
            text = item.get("text") if isinstance(item, dict) else item
            if text is None:
                continue
            label = str(text).strip()
            if label:
                labels.append(label)
        return labels
    return [part.strip() for part in str(value).split(",") if part.strip()]


def normalize_btcpay_record(record):
    """Turn a raw BTCPay JSON/CSV row into the common import-record shape.

    Retains `_btcpay_comment` and `_btcpay_labels` as private keys so the
    coordinator's `apply_btcpay_metadata` step in
    `kassiber.core.imports` can apply them to the inserted transaction
    afterwards.
    """
    sanitized_record = {str(key): value for key, value in record.items() if key is not None}
    txid = sanitized_record.get("TransactionId") or sanitized_record.get("Transaction Id")
    timestamp = sanitized_record.get("Timestamp")
    currency = normalize_asset_code(sanitized_record.get("Currency") or "BTC")
    amount = parse_btcpay_amount(sanitized_record.get("Amount"), currency=currency)
    comment = sanitized_record.get("Comment")
    labels = parse_btcpay_labels(sanitized_record.get("Labels"))
    return {
        "txid": txid,
        "occurred_at": timestamp,
        "confirmed_at": sanitized_record.get("confirmed_at") or sanitized_record.get("ConfirmedAt"),
        "direction": "outbound" if amount < 0 else "inbound",
        "asset": currency,
        "amount": abs(amount),
        "fee": Decimal("0"),
        "fiat_rate": None,
        "fiat_value": None,
        "kind": "withdrawal" if amount < 0 else "deposit",
        "description": comment or "Imported from BTCPay",
        "counterparty": None,
        "_btcpay_comment": comment,
        "_btcpay_labels": labels,
        "raw_json": json.dumps(json_ready(sanitized_record), sort_keys=True),
    }


def load_btcpay_export_records(file_path, input_format):
    """Load a BTCPay export file (JSON list or CSV) and normalize every row."""
    if input_format == "json":
        with open(file_path, "r", encoding="utf-8") as handle:
            payload = json.load(handle)
        if not isinstance(payload, list):
            raise AppError("BTCPay JSON export must be a list of transaction objects")
        rows = payload
    elif input_format == "csv":
        with open(file_path, "r", encoding="utf-8", newline="") as handle:
            rows = list(csv.DictReader(handle))
    else:
        raise AppError(f"Unsupported BTCPay input format '{input_format}'")
    return [normalize_btcpay_record(row) for row in rows]


def is_btcpay_format(input_format):
    return input_format in {"btcpay_json", "btcpay_csv"}


# -- Phoenix -----------------------------------------------------------------


_PHOENIX_REQUIRED_COLUMNS = (
    "date",
    "id",
    "type",
    "amount_msat",
)

_PHOENIX_OUTBOUND_TYPES = {
    "lightning_sent",
    "swap_out",
    "legacy_swap_out",
    "channel_close",
    "liquidity_purchase",
    "fee_bumping",
}

_PHOENIX_INBOUND_TYPES = {
    "lightning_received",
    "swap_in",
    "legacy_swap_in",
    "legacy_pay_to_open",
}


def parse_phoenix_fiat_amount(amount_text):
    """Parse a Phoenix amount_fiat cell like `"22.9998 USD"` into `(Decimal, currency)`."""
    if amount_text is None:
        return None, None
    text = str(amount_text).strip()
    if not text:
        return None, None
    parts = text.split()
    if len(parts) == 1:
        return dec(parts[0]), None
    value = dec(parts[0])
    currency = normalize_asset_code(parts[1])
    return value, currency


def _normalize_payment_hash(value):
    """Normalize a Lightning payment-hash string to 64-char lowercase hex.

    Returns ``None`` when the value is empty, non-hex, or not exactly 32
    bytes. The matcher only trusts values that round-trip cleanly.
    """
    text = str_or_none(value)
    if not text:
        return None
    text = text.strip().lower()
    if len(text) != 64:
        return None
    try:
        bytes.fromhex(text)
    except ValueError:
        return None
    return text


def normalize_phoenix_record(record):
    """Turn a Phoenix CSV row into the common import-record shape.

    Phoenix ships amounts as signed INTEGER msat, fees split across
    `mining_fee_sat` (×1000 to msat) + `service_fee_msat`, and fiat as a
    signed "<value> <CCY>" string. We take the absolute value everywhere
    (direction is captured separately) and derive `fiat_rate` from
    `fiat_value / amount_btc` since Phoenix does not export the rate.

    Lightning rows expose a ``payment_hash`` column that we promote to
    the canonical ``payment_hash`` field consumed by
    ``kassiber.core.imports.normalize_import_record`` so the matcher can
    pair the LN leg of a submarine swap with its on-chain counterpart
    deterministically.

    Private keys `_phoenix_type` and `_phoenix_description` feed
    `apply_phoenix_metadata` in `kassiber.core.imports`.
    """
    sanitized = {str(key): value for key, value in record.items() if key is not None}
    for column in _PHOENIX_REQUIRED_COLUMNS:
        if column not in sanitized:
            raise AppError(f"Phoenix CSV is missing required column '{column}'")
    phoenix_type = str(sanitized.get("type") or "").strip() or "unknown"
    amount_msat_raw = str(sanitized.get("amount_msat") or "0").strip() or "0"
    try:
        amount_msat_signed = int(amount_msat_raw)
    except ValueError as exc:
        raise AppError(f"Invalid Phoenix amount_msat '{amount_msat_raw}'") from exc
    if amount_msat_signed < 0:
        direction = "outbound"
    elif amount_msat_signed > 0:
        direction = "inbound"
    elif phoenix_type in _PHOENIX_OUTBOUND_TYPES:
        direction = "outbound"
    elif phoenix_type in _PHOENIX_INBOUND_TYPES:
        direction = "inbound"
    else:
        direction = "outbound"
    amount_btc = msat_to_btc(abs(amount_msat_signed))
    mining_fee_sat_raw = str(sanitized.get("mining_fee_sat") or "0").strip() or "0"
    service_fee_msat_raw = str(sanitized.get("service_fee_msat") or "0").strip() or "0"
    try:
        mining_fee_msat = int(mining_fee_sat_raw) * 1000
    except ValueError as exc:
        raise AppError(f"Invalid Phoenix mining_fee_sat '{mining_fee_sat_raw}'") from exc
    try:
        service_fee_msat = int(service_fee_msat_raw)
    except ValueError as exc:
        raise AppError(f"Invalid Phoenix service_fee_msat '{service_fee_msat_raw}'") from exc
    fee_btc = msat_to_btc(mining_fee_msat + service_fee_msat)
    fiat_value_signed, _ = parse_phoenix_fiat_amount(sanitized.get("amount_fiat"))
    fiat_value = abs(fiat_value_signed) if fiat_value_signed is not None else None
    fiat_rate = None
    if fiat_value is not None and amount_btc > 0:
        fiat_rate = fiat_value / amount_btc
    description = str_or_none(sanitized.get("description"))
    counterparty = str_or_none(sanitized.get("destination"))
    payment_hash = _normalize_payment_hash(sanitized.get("payment_hash"))
    return {
        "txid": sanitized.get("id"),
        "occurred_at": sanitized.get("date"),
        "direction": direction,
        "asset": "BTC",
        "amount": amount_btc,
        "fee": fee_btc,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "kind": phoenix_type,
        "description": description,
        "counterparty": counterparty,
        "payment_hash": payment_hash,
        "payment_hash_source": "importer" if payment_hash else None,
        "_phoenix_type": phoenix_type,
        "_phoenix_description": description,
        "raw_json": json.dumps(json_ready(sanitized), sort_keys=True),
    }


def load_phoenix_csv_records(file_path):
    """Load a Phoenix CSV export and normalize every row.

    Validates the 4 required columns up-front so the user gets one
    error envelope rather than a cascade of row-level failures.
    """
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = rows[0].keys()
    missing = [column for column in _PHOENIX_REQUIRED_COLUMNS if column not in header]
    if missing:
        raise AppError(
            "Phoenix CSV is missing required columns: " + ", ".join(missing)
        )
    return [normalize_phoenix_record(row) for row in rows]


def is_phoenix_format(input_format):
    return input_format == "phoenix_csv"


# -- River -------------------------------------------------------------------


_RIVER_REQUIRED_COLUMNS = (
    "Date",
    "Sent Amount",
    "Sent Currency",
    "Received Amount",
    "Received Currency",
)


def _normalized_column_key(value):
    return " ".join(str(value).replace("\xa0", " ").strip().split()).casefold()


def _casefold_record(record):
    output = {}
    for key, value in record.items():
        if key is None:
            continue
        output[_normalized_column_key(key)] = value
    return output


def _get_cell(record, *names):
    folded = _casefold_record(record)
    for name in names:
        value = folded.get(_normalized_column_key(name))
        if value not in (None, ""):
            return value
    return None


def _clean_decimal_text(value):
    if value in (None, ""):
        return None
    text = str(value).strip()
    if not text:
        return None
    negative = False
    if text.startswith("(") and text.endswith(")"):
        negative = True
        text = text[1:-1].strip()
    for symbol in ("$", "€", "£", "CHF", "USD", "EUR", "GBP", "BTC", "XBT", "LBTC"):
        text = text.replace(symbol, "")
    text = text.replace(",", "").replace(" ", "").strip()
    if not text:
        return None
    if negative and not text.startswith("-"):
        text = "-" + text
    return text


def _decimal_cell(value):
    text = _clean_decimal_text(value)
    return dec(text) if text is not None else None


def _currency_cell(value):
    currency = str_or_none(value)
    return normalize_asset_code(currency) if currency else None


def _river_decimal(value):
    return _decimal_cell(value)


def _river_currency(value):
    return _currency_cell(value)


# -- Bull Bitcoin ------------------------------------------------------------


_BULLBITCOIN_REQUIRED_COLUMNS = (
    "ORDER_STATUS",
    "PAYIN_AMOUNT",
    "PAYIN_CURRENCY",
    "PAYOUT_AMOUNT",
    "PAYOUT_CURRENCY",
    "COMPLETED_AT (UTC)",
    "TRANSACTION_ID",
)

_BULLBITCOIN_CRYPTO_CURRENCIES = {"BTC", "XBT", "LBTC"}


def _bullbitcoin_completed(record):
    for column in ("ORDER_STATUS", "PAYIN_STATUS", "PAYOUT_STATUS"):
        value = str_or_none(_get_cell(record, column))
        if value and value.strip().casefold() != "completed":
            return False
    return True


def normalize_bullbitcoin_record(record):
    """Turn one Bull Bitcoin order-export row into the common import shape."""
    sanitized = {
        str(key).strip(): value for key, value in record.items() if key is not None
    }
    if not _bullbitcoin_completed(sanitized):
        return None

    payin_currency = _currency_cell(_get_cell(sanitized, "PAYIN_CURRENCY"))
    payout_currency = _currency_cell(_get_cell(sanitized, "PAYOUT_CURRENCY"))
    payin_amount = _decimal_cell(_get_cell(sanitized, "PAYIN_AMOUNT"))
    payout_amount = _decimal_cell(_get_cell(sanitized, "PAYOUT_AMOUNT"))
    if payin_amount is None or payout_amount is None:
        raise AppError("Bull Bitcoin CSV has a completed row with an empty amount")

    payin_is_crypto = payin_currency in _BULLBITCOIN_CRYPTO_CURRENCIES
    payout_is_crypto = payout_currency in _BULLBITCOIN_CRYPTO_CURRENCIES
    if payin_is_crypto and not payout_is_crypto:
        direction = "outbound"
        asset = "BTC" if payin_currency == "XBT" else payin_currency
        amount = abs(payin_amount)
        fiat_value = abs(payout_amount)
        fiat_currency = payout_currency
        kind = "sell"
    elif payout_is_crypto and not payin_is_crypto:
        direction = "inbound"
        asset = "BTC" if payout_currency == "XBT" else payout_currency
        amount = abs(payout_amount)
        fiat_value = abs(payin_amount)
        fiat_currency = payin_currency
        kind = "buy"
    else:
        return None

    txid = str_or_none(_get_cell(sanitized, "TRANSACTION_ID"))
    if not txid:
        return None
    exchange_rate = _decimal_cell(_get_cell(sanitized, "EXCHANGE_RATE_AMOUNT"))
    fiat_rate = exchange_rate if exchange_rate is not None else fiat_value / amount
    order_ref = (
        str_or_none(_get_cell(sanitized, "ORDER_ID"))
        or str_or_none(_get_cell(sanitized, "ORDER_NUMBER"))
        or txid
    )
    occurred_at = (
        _get_cell(sanitized, "COMPLETED_AT (UTC)")
        or _get_cell(sanitized, "SENT_AT (UTC)")
        or _get_cell(sanitized, "CREATED_AT (UTC)")
    )
    payin_method = str_or_none(_get_cell(sanitized, "PAYIN_METHOD"))
    payout_method = str_or_none(_get_cell(sanitized, "PAYOUT_METHOD"))
    method_text = " to ".join(part for part in (payin_method, payout_method) if part)
    description = f"Bull Bitcoin {kind}"
    if method_text:
        description = f"{description} - {method_text}"
    return {
        "txid": txid,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": Decimal("0"),
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "fiat_currency": fiat_currency,
        "pricing_source_kind": "exchange_execution",
        "pricing_provider": "Bull Bitcoin",
        "pricing_pair": f"{asset}-{fiat_currency}" if fiat_currency else None,
        "pricing_method": "bullbitcoin_csv",
        "pricing_external_ref": order_ref,
        "pricing_quality": "exact",
        "kind": kind,
        "description": description,
        "counterparty": "Bull Bitcoin",
        "raw_json": json.dumps(json_ready(sanitized), sort_keys=True),
    }


def load_bullbitcoin_csv_records(file_path):
    """Load Bull Bitcoin order-export CSV rows."""
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = {_normalized_column_key(column) for column in rows[0].keys()}
    missing = [
        column for column in _BULLBITCOIN_REQUIRED_COLUMNS if _normalized_column_key(column) not in header
    ]
    if missing:
        raise AppError("Bull Bitcoin CSV is missing required columns: " + ", ".join(missing))
    normalized = []
    for row in rows:
        record = normalize_bullbitcoin_record(row)
        if record is not None:
            normalized.append(record)
    return normalized


def is_bullbitcoin_format(input_format):
    return input_format == "bullbitcoin_csv"


# -- Coinfinity --------------------------------------------------------------


_COINFINITY_REQUIRED_COLUMNS = (
    "Order ID",
    "Type",
    "Date",
    "Amount EUR",
    "Amount Crypto",
    "Crypto",
    "Rate EUR",
    "Mining Fee Crypto",
    "Total Fee EUR",
    "Transaction",
)

_COINFINITY_CRYPTO_CURRENCIES = {"BTC", "XBT"}


def _coinfinity_user_direction(order_type):
    """Map Coinfinity's broker-side type to the user's BTC movement."""
    value = str(order_type or "").strip().casefold()
    if value == "sell":
        return "inbound", "buy"
    if value == "buy":
        return "outbound", "sell"
    return None, None


def normalize_coinfinity_record(record, index=0):
    """Turn one Coinfinity order-export row into the common import shape."""
    sanitized = {str(key).strip(): value for key, value in record.items() if key is not None}
    direction, kind = _coinfinity_user_direction(_get_cell(sanitized, "Type"))
    if direction is None or kind is None:
        return None

    currency = _currency_cell(_get_cell(sanitized, "Crypto"))
    if currency not in _COINFINITY_CRYPTO_CURRENCIES:
        return None
    asset = "BTC" if currency == "XBT" else currency
    amount = _decimal_cell(_get_cell(sanitized, "Amount Crypto"))
    fiat_amount = _decimal_cell(_get_cell(sanitized, "Amount EUR"))
    fee_fiat = _decimal_cell(_get_cell(sanitized, "Total Fee EUR")) or Decimal("0")
    if amount is None or fiat_amount is None:
        raise AppError("Coinfinity CSV has a BTC/EUR row with an empty amount")

    if direction == "inbound":
        fiat_value = abs(fiat_amount) + abs(fee_fiat)
        btc_fee = Decimal("0")
    else:
        fiat_value = max(Decimal("0"), abs(fiat_amount) - abs(fee_fiat))
        btc_fee = _decimal_cell(_get_cell(sanitized, "Mining Fee Crypto")) or Decimal(
            "0"
        )

    fiat_rate = _decimal_cell(_get_cell(sanitized, "Rate EUR"))
    occurred_at = _get_cell(sanitized, "Date")
    order_ref = str_or_none(_get_cell(sanitized, "Order ID")) or (
        f"coinfinity:{occurred_at}:{direction}:{asset}:{amount}:{index}"
    )
    transaction_ref = str_or_none(_get_cell(sanitized, "Transaction"))
    lightning_ref = str_or_none(_get_cell(sanitized, "LN Invoice"))
    external_id = transaction_ref or lightning_ref or f"coinfinity:{order_ref}"
    transaction_type = str_or_none(_get_cell(sanitized, "Transaction type"))
    description = f"Coinfinity {kind}"
    if transaction_type:
        description = f"{description} - {transaction_type}"
    payload = {
        "txid": external_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": abs(amount),
        "fee": abs(btc_fee),
        "fiat_rate": (
            fiat_rate
            if fiat_rate is not None
            else (fiat_value / abs(amount) if amount else None)
        ),
        "fiat_value": fiat_value,
        "fiat_currency": "EUR",
        "pricing_source_kind": "exchange_execution",
        "pricing_provider": "Coinfinity",
        "pricing_pair": f"{asset}-EUR",
        "pricing_timestamp": occurred_at,
        "pricing_method": "coinfinity_csv",
        "pricing_external_ref": order_ref,
        "pricing_quality": "exact",
        "kind": kind,
        "description": description,
        "counterparty": "Coinfinity",
        "raw_json": json.dumps(json_ready(sanitized), sort_keys=True),
    }
    if not transaction_ref and not lightning_ref:
        payload["_exchange_evidence_match_by_economics"] = True
        payload["_exchange_evidence_match_time_tolerance_seconds"] = 172800
    return payload


def load_coinfinity_csv_records(file_path):
    """Load Coinfinity order-export CSV rows."""
    with open(file_path, "r", encoding="utf-8-sig", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = {_normalized_column_key(column) for column in rows[0].keys()}
    missing = [
        column
        for column in _COINFINITY_REQUIRED_COLUMNS
        if _normalized_column_key(column) not in header
    ]
    if missing:
        raise AppError("Coinfinity CSV is missing required columns: " + ", ".join(missing))
    normalized = []
    for index, row in enumerate(rows, start=1):
        record = normalize_coinfinity_record(row, index=index)
        if record is not None:
            normalized.append(record)
    return normalized


def is_coinfinity_format(input_format):
    return input_format == "coinfinity_csv"


# -- Pocket Bitcoin ----------------------------------------------------------


_POCKETBITCOIN_REQUIRED_COLUMNS = (
    "type",
    "date",
    "reference",
    "price.currency",
    "price.amount",
    "cost.currency",
    "cost.amount",
    "fee.currency",
    "fee.amount",
    "value.currency",
    "value.amount",
)

_POCKETBITCOIN_CRYPTO_CURRENCIES = {"BTC", "XBT", "LBTC"}


def _pocketbitcoin_row_type(record):
    return str(_get_cell(record, "type") or "").strip().casefold()


def _pocketbitcoin_crypto_asset(currency):
    if currency in _POCKETBITCOIN_CRYPTO_CURRENCIES:
        return "BTC" if currency == "XBT" else currency
    return None


def _pocketbitcoin_reference(record, fallback):
    return str_or_none(_get_cell(record, "reference")) or fallback


def _pocketbitcoin_withdrawal_key(withdrawal):
    currency = _currency_cell(_get_cell(withdrawal, "value.currency"))
    asset = _pocketbitcoin_crypto_asset(currency)
    amount = _decimal_cell(_get_cell(withdrawal, "value.amount"))
    fee = _decimal_cell(_get_cell(withdrawal, "fee.amount"))
    fee_currency = _currency_cell(_get_cell(withdrawal, "fee.currency"))
    if asset is None or amount is None or fee is None or fee_currency != currency:
        return None
    return asset, abs(amount) + abs(fee)


def _pocketbitcoin_withdrawal_pairs(rows):
    """Match Pocket exchange gross BTC amounts to separate withdrawal rows."""
    withdrawals_by_key = {}
    for row in rows:
        if _pocketbitcoin_row_type(row) != "withdrawal":
            continue
        key = _pocketbitcoin_withdrawal_key(row)
        if key is None:
            continue
        withdrawals_by_key.setdefault(key, []).append(row)
    for bucket in withdrawals_by_key.values():
        bucket.sort(key=lambda item: str(_get_cell(item, "date") or ""))

    pairs = {}
    for row in rows:
        if _pocketbitcoin_row_type(row) != "exchange":
            continue
        value_currency = _currency_cell(_get_cell(row, "value.currency"))
        asset = _pocketbitcoin_crypto_asset(value_currency)
        gross_amount = _decimal_cell(_get_cell(row, "value.amount"))
        if asset is None or gross_amount is None:
            continue
        candidates = withdrawals_by_key.get((asset, abs(gross_amount)), [])
        if not candidates:
            continue
        exchange_date = str(_get_cell(row, "date") or "")
        chosen_index = None
        for index, candidate in enumerate(candidates):
            withdrawal_date = str(_get_cell(candidate, "date") or "")
            if not exchange_date or not withdrawal_date or withdrawal_date >= exchange_date:
                chosen_index = index
                break
        if chosen_index is None:
            chosen_index = 0
        pairs[id(row)] = candidates.pop(chosen_index)
    return pairs


def normalize_pocketbitcoin_record(record, withdrawal=None, index=0):
    """Turn one Pocket Bitcoin exchange row into the common import shape."""
    sanitized = {str(key).strip(): value for key, value in record.items() if key is not None}
    if _pocketbitcoin_row_type(sanitized) != "exchange":
        return None

    cost_currency = _currency_cell(_get_cell(sanitized, "cost.currency"))
    value_currency = _currency_cell(_get_cell(sanitized, "value.currency"))
    fee_currency = _currency_cell(_get_cell(sanitized, "fee.currency"))
    cost_amount = _decimal_cell(_get_cell(sanitized, "cost.amount"))
    value_amount = _decimal_cell(_get_cell(sanitized, "value.amount"))
    fee_amount = _decimal_cell(_get_cell(sanitized, "fee.amount"))
    if cost_amount is None or value_amount is None:
        raise AppError("Pocket Bitcoin CSV has an exchange row with an empty amount")

    cost_asset = _pocketbitcoin_crypto_asset(cost_currency)
    value_asset = _pocketbitcoin_crypto_asset(value_currency)
    if value_asset and not cost_asset:
        direction = "inbound"
        asset = value_asset
        amount = abs(value_amount)
        fiat_value = abs(cost_amount)
        fiat_currency = cost_currency
        kind = "buy"
        if fee_currency == fiat_currency and fee_amount is not None:
            fiat_value += abs(fee_amount)
    elif cost_asset and not value_asset:
        direction = "outbound"
        asset = cost_asset
        amount = abs(cost_amount)
        fiat_value = abs(value_amount)
        fiat_currency = value_currency
        kind = "sell"
        if fee_currency == fiat_currency and fee_amount is not None:
            fiat_value = max(Decimal("0"), fiat_value - abs(fee_amount))
    else:
        return None

    btc_fee = Decimal("0")
    occurred_at = _get_cell(sanitized, "date")
    raw_payload = dict(sanitized)
    if withdrawal is not None:
        withdrawal_currency = _currency_cell(_get_cell(withdrawal, "value.currency"))
        withdrawal_asset = _pocketbitcoin_crypto_asset(withdrawal_currency)
        withdrawal_amount = _decimal_cell(_get_cell(withdrawal, "value.amount"))
        withdrawal_fee = _decimal_cell(_get_cell(withdrawal, "fee.amount"))
        withdrawal_fee_currency = _currency_cell(_get_cell(withdrawal, "fee.currency"))
        if (
            direction == "inbound"
            and withdrawal_asset == asset
            and withdrawal_amount is not None
        ):
            amount = abs(withdrawal_amount)
            occurred_at = _get_cell(withdrawal, "date") or occurred_at
            if withdrawal_fee_currency == withdrawal_currency and withdrawal_fee is not None:
                btc_fee = abs(withdrawal_fee)
        raw_payload["_pocketbitcoin_withdrawal"] = {
            str(key).strip(): value for key, value in withdrawal.items() if key is not None
        }

    price_amount = _decimal_cell(_get_cell(sanitized, "price.amount"))
    fiat_rate = price_amount if price_amount is not None else (fiat_value / amount if amount else None)
    reference = _pocketbitcoin_reference(
        sanitized,
        f"pocketbitcoin:{occurred_at}:{direction}:{asset}:{amount}:{index}",
    )
    description = f"Pocket Bitcoin {kind}"
    return {
        "txid": f"pocketbitcoin:{reference}",
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": btc_fee,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "fiat_currency": fiat_currency,
        "pricing_source_kind": "exchange_execution",
        "pricing_provider": "Pocket Bitcoin",
        "pricing_pair": f"{asset}-{fiat_currency}" if fiat_currency else None,
        "pricing_timestamp": _get_cell(sanitized, "date"),
        "pricing_method": "pocketbitcoin_csv",
        "pricing_external_ref": reference,
        "pricing_quality": "exact",
        "kind": kind,
        "description": description,
        "counterparty": "Pocket Bitcoin",
        "_exchange_evidence_match_by_economics": True,
        "_exchange_evidence_match_time_tolerance_seconds": 172800,
        "raw_json": json.dumps(json_ready(raw_payload), sort_keys=True),
    }


def load_pocketbitcoin_csv_records(file_path):
    """Load Pocket Bitcoin account CSV rows."""
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = {_normalized_column_key(column) for column in rows[0].keys()}
    missing = [
        column
        for column in _POCKETBITCOIN_REQUIRED_COLUMNS
        if _normalized_column_key(column) not in header
    ]
    if missing:
        raise AppError("Pocket Bitcoin CSV is missing required columns: " + ", ".join(missing))
    withdrawals = _pocketbitcoin_withdrawal_pairs(rows)
    normalized = []
    for index, row in enumerate(rows, start=1):
        record = normalize_pocketbitcoin_record(row, withdrawals.get(id(row)), index=index)
        if record is not None:
            normalized.append(record)
    return normalized


def is_pocketbitcoin_format(input_format):
    return input_format == "pocketbitcoin_csv"


# -- 21bitcoin ---------------------------------------------------------------


_TWENTYONEBITCOIN_REQUIRED_COLUMNS = (
    "id",
    "transaction_date",
    "buy_asset",
    "buy_amount",
    "sell_asset",
    "sell_amount",
    "fee_asset",
    "fee_amount",
    "transaction_type",
)

_TWENTYONEBITCOIN_CRYPTO_CURRENCIES = {"BTC", "XBT", "LBTC"}


def _twentyonebitcoin_asset(value):
    asset = _currency_cell(value)
    return "BTC" if asset == "XBT" else asset


def _twentyonebitcoin_datetime(value):
    text = str_or_none(value)
    if not text:
        return None
    text = text.strip()
    date_part, _, time_part = text.partition(" ")
    date_bits = date_part.split(".")
    if len(date_bits) != 3:
        return text
    day, month, year_text = date_bits
    year_text = year_text.strip()
    time_bits = (time_part.strip() or "00:00:00").split(":")
    if len(time_bits) != 3:
        return text
    try:
        if len(year_text) == 3 and year_text.startswith("2"):
            year = 2000 + int(year_text[-2:])
        elif len(year_text) == 2:
            year = 2000 + int(year_text)
        else:
            year = int(year_text)
        second = time_bits[2].split(".", 1)[0]
        dt = datetime(
            year,
            int(month),
            int(day),
            int(time_bits[0]),
            int(time_bits[1]),
            int(second),
            tzinfo=timezone.utc,
        )
    except ValueError:
        return text
    return dt.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def normalize_twentyonebitcoin_record(record):
    """Turn one 21bitcoin transaction-export row into the common import shape."""
    sanitized = {str(key).strip(): value for key, value in record.items() if key is not None}
    row_id = str_or_none(_get_cell(sanitized, "id"))
    if not row_id:
        raise AppError("21bitcoin CSV has a BTC-side row without an id")

    buy_currency = _twentyonebitcoin_asset(_get_cell(sanitized, "buy_asset"))
    sell_currency = _twentyonebitcoin_asset(_get_cell(sanitized, "sell_asset"))
    fee_currency = _twentyonebitcoin_asset(_get_cell(sanitized, "fee_asset"))
    buy_amount = _decimal_cell(_get_cell(sanitized, "buy_amount"))
    sell_amount = _decimal_cell(_get_cell(sanitized, "sell_amount"))
    fee_amount = abs(_decimal_cell(_get_cell(sanitized, "fee_amount")) or Decimal("0"))

    buy_is_crypto = buy_currency in _TWENTYONEBITCOIN_CRYPTO_CURRENCIES
    sell_is_crypto = sell_currency in _TWENTYONEBITCOIN_CRYPTO_CURRENCIES
    fee_is_crypto = fee_currency in _TWENTYONEBITCOIN_CRYPTO_CURRENCIES
    fee_is_fiat = fee_currency is not None and not fee_is_crypto
    transaction_type = str_or_none(_get_cell(sanitized, "transaction_type")) or "transaction"
    transaction_type_key = transaction_type.strip().lower()
    is_l1_withdrawal = transaction_type_key == "withdrawal"

    fiat_currency = None
    fiat_value = None
    fiat_rate = None
    pricing_source_kind = None
    pricing_quality = None
    if buy_is_crypto and not sell_is_crypto:
        if buy_amount is None:
            raise AppError("21bitcoin CSV has a BTC buy row with an empty buy_amount")
        direction = "inbound"
        asset = buy_currency
        amount = abs(buy_amount)
        fee = fee_amount if fee_currency == asset else Decimal("0")
        if sell_amount is not None and sell_currency:
            fiat_currency = sell_currency
            fiat_value = abs(sell_amount) + (fee_amount if fee_is_fiat else Decimal("0"))
        kind = "buy" if sell_currency else _twentyonebitcoin_kind(transaction_type, direction)
    elif sell_is_crypto and not buy_is_crypto:
        if sell_amount is None:
            raise AppError("21bitcoin CSV has a BTC sell/withdrawal row with an empty sell_amount")
        direction = "outbound"
        asset = sell_currency
        amount = abs(sell_amount)
        fee = fee_amount if fee_currency == asset else Decimal("0")
        if buy_amount is not None and buy_currency:
            fiat_currency = buy_currency
            fiat_value = max(Decimal("0"), abs(buy_amount) - (fee_amount if fee_is_fiat else Decimal("0")))
            kind = "sell"
        else:
            kind = _twentyonebitcoin_kind(transaction_type, direction)
    else:
        return None

    if fiat_value is not None and amount > 0:
        fiat_rate = fiat_value / amount
        pricing_source_kind = "exchange_execution"
        pricing_quality = "exact"

    note = str_or_none(_get_cell(sanitized, "note"))
    depot = str_or_none(_get_cell(sanitized, "depot_name"))
    description_parts = [part for part in (transaction_type, note, depot) if part]
    description = " - ".join(description_parts) or "Imported from 21bitcoin"
    provider_ref = f"21bitcoin:{row_id}"
    linked_transaction = str_or_none(_get_cell(sanitized, "linked_transaction"))
    external_ref = linked_transaction if is_l1_withdrawal and linked_transaction else provider_ref
    return {
        "txid": external_ref,
        "occurred_at": _twentyonebitcoin_datetime(_get_cell(sanitized, "transaction_date")),
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "fiat_currency": fiat_currency,
        "pricing_source_kind": pricing_source_kind,
        "pricing_provider": "21bitcoin",
        "pricing_pair": f"{asset}-{fiat_currency}" if fiat_currency else None,
        "pricing_method": "21bitcoin_csv" if pricing_source_kind else None,
        "pricing_external_ref": row_id,
        "pricing_quality": pricing_quality,
        "kind": kind,
        "description": description,
        "counterparty": str_or_none(_get_cell(sanitized, "exchange_name")) or "21bitcoin",
        "_exchange_evidence_matchable": is_l1_withdrawal,
        "_21bitcoin_l1_withdrawal": is_l1_withdrawal,
        "raw_json": json.dumps(json_ready(sanitized), sort_keys=True),
    }


def _twentyonebitcoin_kind(transaction_type, direction):
    tag = str(transaction_type or "").strip().lower()
    aliases = {
        "deposit": "deposit",
        "trade": "buy" if direction == "inbound" else "sell",
        "withdrawal": "withdrawal",
    }
    return aliases.get(tag, tag.replace(" ", "_") if tag else direction)


def load_twentyonebitcoin_csv_records(file_path):
    """Load 21bitcoin transaction-export CSV rows."""
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = {_normalized_column_key(column) for column in rows[0].keys()}
    missing = [
        column
        for column in _TWENTYONEBITCOIN_REQUIRED_COLUMNS
        if _normalized_column_key(column) not in header
    ]
    if missing:
        raise AppError("21bitcoin CSV is missing required columns: " + ", ".join(missing))
    normalized = []
    for row in rows:
        record = normalize_twentyonebitcoin_record(row)
        if record is not None:
            normalized.append(record)
    return normalized


def is_twentyonebitcoin_format(input_format):
    return input_format == "21bitcoin_csv"


# -- Strike ------------------------------------------------------------------


_STRIKE_REQUIRED_COLUMNS = (
    "Reference",
    "Date & Time (UTC)",
    "Transaction Type",
    "Amount BTC",
)


def _strike_datetime(value):
    text = str_or_none(value)
    if not text:
        return None
    text = text.strip()
    for fmt in ("%b %d %Y %H:%M:%S", "%B %d %Y %H:%M:%S"):
        try:
            dt = datetime.strptime(text, fmt).replace(tzinfo=timezone.utc)
        except ValueError:
            continue
        return dt.replace(microsecond=0).isoformat().replace("+00:00", "Z")
    return text


def _strike_fiat_currency(record):
    # Strike exports one fiat column family per CSV; pick that fiat from amount,
    # fee, or cost-basis headers and ignore timestamp suffixes such as "(UTC)".
    for key in record.keys():
        label = str(key or "").strip()
        label_key = label.casefold()
        if label_key in {"btc price", "amount btc", "fee btc"}:
            continue
        if "(" in label and ")" in label and label_key.startswith(("amount", "fee", "cost basis")):
            candidate = label.rsplit("(", 1)[1].split(")", 1)[0]
            currency = _currency_cell(candidate)
            if currency and currency not in {"BTC", "XBT", "LBTC"}:
                return currency
        parts = label.split()
        if len(parts) >= 2 and parts[0].casefold() in {"amount", "fee"}:
            currency = _currency_cell(parts[-1])
            if currency and currency not in {"BTC", "XBT", "LBTC"}:
                return currency
    return None


def _strike_lightning_row(transaction_type, destination):
    text = f"{transaction_type or ''} {destination or ''}".strip().casefold()
    destination_text = str(destination or "").strip().casefold()
    return (
        "lightning" in text
        or destination_text.startswith(("lnbc", "lntb", "lnbcrt", "lightning:"))
    )


def _strike_payment_hash(value, *, lightning):
    if not lightning:
        return None
    return _normalize_payment_hash(value)


def _strike_decimal_cell(value):
    text = str_or_none(value)
    if text and text.strip() in {"-", "–", "—"}:
        return None
    return _decimal_cell(value)


def _strike_kind(transaction_type, direction):
    tag = str(transaction_type or "").strip().casefold()
    aliases = {
        "buy": "buy",
        "purchase": "buy",
        "sell": "sell",
        "receive": "receive",
        "received": "receive",
        "send": "send",
        "sent": "send",
        "withdraw": "withdrawal",
        "withdrawal": "withdrawal",
    }
    return aliases.get(tag, tag.replace(" ", "_") if tag else direction)


def _strike_fiat_amount(record, fiat_currency):
    if not fiat_currency:
        return None
    return _strike_decimal_cell(
        _get_cell(
            record,
            f"Amount {fiat_currency}",
            f"Amount ({fiat_currency})",
        )
    )


def _strike_fiat_fee(record, fiat_currency):
    if not fiat_currency:
        return Decimal("0")
    return abs(
        _strike_decimal_cell(
            _get_cell(
                record,
                f"Fee {fiat_currency}",
                f"Fee ({fiat_currency})",
            )
        )
        or Decimal("0")
    )


def _strike_cost_basis(record, fiat_currency):
    if not fiat_currency:
        return None
    return _strike_decimal_cell(
        _get_cell(
            record,
            f"Cost Basis ({fiat_currency})",
            f"Cost Basis {fiat_currency}",
        )
    )


def normalize_strike_record(record):
    """Turn one Strike export row into the common import shape."""
    sanitized = {str(key).strip(): value for key, value in record.items() if key is not None}
    row_ref = str_or_none(_get_cell(sanitized, "Reference"))
    if not row_ref:
        raise AppError("Strike CSV has a BTC-side row without a Reference")

    amount_btc = _strike_decimal_cell(_get_cell(sanitized, "Amount BTC"))
    if amount_btc in (None, Decimal("0")):
        return None
    direction = "outbound" if amount_btc < 0 else "inbound"
    amount = abs(amount_btc)
    fee = abs(_strike_decimal_cell(_get_cell(sanitized, "Fee BTC")) or Decimal("0"))

    fiat_currency = _strike_fiat_currency(sanitized)
    amount_fiat = _strike_fiat_amount(sanitized, fiat_currency)
    fee_fiat = _strike_fiat_fee(sanitized, fiat_currency)
    cost_basis = _strike_cost_basis(sanitized, fiat_currency)
    fiat_rate = abs(_strike_decimal_cell(_get_cell(sanitized, "BTC Price")) or Decimal("0")) or None
    transaction_type = str_or_none(_get_cell(sanitized, "Transaction Type")) or "transaction"
    destination = str_or_none(_get_cell(sanitized, "Destination"))
    description = str_or_none(_get_cell(sanitized, "Description"))
    note = str_or_none(_get_cell(sanitized, "Note"))
    transaction_hash = str_or_none(_get_cell(sanitized, "Transaction Hash"))
    lightning = _strike_lightning_row(transaction_type, destination)
    payment_hash = _strike_payment_hash(transaction_hash, lightning=lightning)
    external_ref = f"strike:{row_ref}" if lightning or not transaction_hash else transaction_hash
    kind = _strike_kind(transaction_type, direction)

    fiat_value = None
    if amount_fiat is not None:
        if direction == "inbound" and kind == "buy":
            fiat_value = abs(amount_fiat) + fee_fiat
        elif direction == "outbound" and kind == "sell":
            fiat_value = max(Decimal("0"), abs(amount_fiat) - fee_fiat)
        else:
            fiat_value = abs(amount_fiat)
    elif fiat_rate is not None:
        fiat_value = amount * fiat_rate
    elif kind == "buy" and cost_basis is not None:
        fiat_value = abs(cost_basis)

    if fiat_rate is None and fiat_value is not None and amount > 0:
        fiat_rate = fiat_value / amount
    pricing_source_kind = "exchange_execution" if fiat_rate is not None or fiat_value is not None else None
    summary = " - ".join(part for part in (transaction_type, description, note) if part)
    raw_payload = dict(sanitized)
    raw_payload["_strike_lightning"] = lightning
    return {
        "txid": external_ref,
        "occurred_at": _strike_datetime(_get_cell(sanitized, "Date & Time (UTC)")),
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "fiat_currency": fiat_currency,
        "pricing_source_kind": pricing_source_kind,
        "pricing_provider": "Strike" if pricing_source_kind else None,
        "pricing_pair": f"BTC-{fiat_currency}" if fiat_currency and pricing_source_kind else None,
        "pricing_timestamp": _strike_datetime(_get_cell(sanitized, "Date & Time (UTC)"))
        if pricing_source_kind
        else None,
        "pricing_method": "strike_csv" if pricing_source_kind else None,
        "pricing_external_ref": row_ref if pricing_source_kind else None,
        "pricing_quality": "exact" if pricing_source_kind else None,
        "kind": kind,
        "description": summary or "Imported from Strike",
        "counterparty": "Strike",
        "payment_hash": payment_hash,
        "payment_hash_source": "importer" if payment_hash else None,
        "raw_json": json.dumps(json_ready(raw_payload), sort_keys=True),
    }


def load_strike_csv_records(file_path):
    """Load Strike transaction-history CSV rows."""
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = {_normalized_column_key(column) for column in rows[0].keys()}
    missing = [
        column
        for column in _STRIKE_REQUIRED_COLUMNS
        if _normalized_column_key(column) not in header
    ]
    if missing:
        raise AppError("Strike CSV is missing required columns: " + ", ".join(missing))
    normalized = []
    for row in rows:
        record = normalize_strike_record(row)
        if record is not None:
            normalized.append(record)
    return normalized


def is_strike_format(input_format):
    return input_format == "strike_csv"


def is_exchange_evidence_format(input_format):
    return input_format in {
        "bullbitcoin_csv",
        "coinfinity_csv",
        "21bitcoin_csv",
        "pocketbitcoin_csv",
    }


def exchange_evidence_label(input_format):
    if input_format == "bullbitcoin_csv":
        return "Bull Bitcoin"
    if input_format == "coinfinity_csv":
        return "Coinfinity"
    if input_format == "21bitcoin_csv":
        return "21bitcoin"
    if input_format == "pocketbitcoin_csv":
        return "Pocket Bitcoin"
    return str(input_format or "").replace("_", " ")


def exchange_evidence_rows_key(input_format):
    if input_format == "bullbitcoin_csv":
        return "bullbitcoin_rows"
    if input_format == "coinfinity_csv":
        return "coinfinity_rows"
    if input_format == "21bitcoin_csv":
        return "twentyonebitcoin_rows"
    if input_format == "pocketbitcoin_csv":
        return "pocketbitcoin_rows"
    return "exchange_rows"


# -- River -------------------------------------------------------------------


def _river_fiat_amount(value, currency):
    if not currency or currency in {"BTC", "XBT"}:
        return None
    amount = _river_decimal(value)
    return abs(amount) if amount is not None else None


def _river_btc_amount(value, currency):
    if currency not in {"BTC", "XBT"}:
        return None
    amount = _river_decimal(value)
    return abs(amount) if amount is not None else None


def _river_kind(value):
    tag = str(value or "").strip().lower()
    aliases = {
        "automatic withdrawal": "withdrawal",
        "cash deposit": "cash_deposit",
        "cash withdrawal": "cash_withdrawal",
        "mining payout": "mining",
        "referral reward": "income",
        "bitcoin interest on cash": "interest",
    }
    return aliases.get(tag, tag.replace(" ", "_") if tag else "river_activity")


def normalize_river_record(record):
    """Turn one River CSV row into the common import-record shape.

    River currently documents two exports. Bitcoin Activity has the BTC/cash
    legs and a `Tag`; Account Activity adds reference code, transaction type,
    method/source/destination, price, and on-chain transaction id. Fiat-only
    rows are skipped by returning `None`; Kassiber is the BTC-side subledger.
    """
    sanitized = {str(key).strip(): value for key, value in record.items() if key is not None}
    sent_currency = _river_currency(_get_cell(sanitized, "Sent Currency"))
    received_currency = _river_currency(_get_cell(sanitized, "Received Currency"))
    fee_currency = _river_currency(_get_cell(sanitized, "Fee Currency"))
    price_currency = _river_currency(_get_cell(sanitized, "Bitcoin Price Currency"))
    sent_btc = _river_btc_amount(_get_cell(sanitized, "Sent Amount"), sent_currency)
    received_btc = _river_btc_amount(_get_cell(sanitized, "Received Amount"), received_currency)
    sent_fiat = _river_fiat_amount(_get_cell(sanitized, "Sent Amount"), sent_currency)
    received_fiat = _river_fiat_amount(_get_cell(sanitized, "Received Amount"), received_currency)
    fee_btc = _river_btc_amount(_get_cell(sanitized, "Fee Amount"), fee_currency) or Decimal("0")
    fee_fiat = _river_fiat_amount(_get_cell(sanitized, "Fee Amount"), fee_currency) or Decimal("0")
    price = _river_decimal(_get_cell(sanitized, "Bitcoin Price Amount"))
    reference = str_or_none(_get_cell(sanitized, "Transaction ID", "Reference Code"))
    tag = str_or_none(_get_cell(sanitized, "Tag", "Transaction Type"))
    transaction_type = str_or_none(_get_cell(sanitized, "Transaction Type")) or tag

    if received_btc is not None:
        direction = "inbound"
        amount = received_btc
        if sent_fiat is not None:
            fiat_value = sent_fiat + fee_fiat
            cash_leg_pricing = True
            kind = "buy"
        else:
            fiat_value = amount * price if price is not None else None
            cash_leg_pricing = False
            kind = _river_kind(transaction_type)
    elif sent_btc is not None:
        direction = "outbound"
        amount = sent_btc
        if received_fiat is not None:
            fiat_value = max(Decimal("0"), received_fiat - fee_fiat)
            cash_leg_pricing = True
            kind = "sell"
        else:
            fiat_value = amount * price if price is not None else None
            cash_leg_pricing = False
            kind = _river_kind(transaction_type)
    else:
        return None

    fiat_rate = fiat_value / amount if fiat_value is not None and amount > 0 else price
    fiat_currency = (
        sent_currency
        if sent_fiat is not None
        else received_currency
        if received_fiat is not None
        else price_currency
    )
    source = str_or_none(_get_cell(sanitized, "Source"))
    destination = str_or_none(_get_cell(sanitized, "Destination"))
    method = str_or_none(_get_cell(sanitized, "Method"))
    description_parts = [part for part in (transaction_type, method, source, destination) if part]
    description = " - ".join(description_parts) or "Imported from River"
    payment_hash = _normalize_payment_hash(_get_cell(sanitized, "Payment Hash", "payment_hash"))
    return {
        "txid": reference,
        "occurred_at": _get_cell(sanitized, "Date"),
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee_btc,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "fiat_currency": fiat_currency,
        "pricing_source_kind": "exchange_execution"
        if cash_leg_pricing
        else "fmv_provider"
        if fiat_value is not None
        else None,
        "pricing_provider": "River",
        "pricing_pair": f"BTC-{fiat_currency}" if fiat_currency else None,
        "pricing_method": "river_csv",
        "pricing_external_ref": reference,
        "pricing_quality": "exact"
        if cash_leg_pricing
        else "provider_sample"
        if fiat_value is not None
        else None,
        "kind": kind,
        "description": description,
        "counterparty": destination if direction == "outbound" else source,
        "payment_hash": payment_hash,
        "payment_hash_source": "importer" if payment_hash else None,
        "_river_tag": tag,
        "_river_description": description,
        "raw_json": json.dumps(json_ready(sanitized), sort_keys=True),
    }


def load_river_csv_records(file_path):
    """Load River Bitcoin Activity or Account Activity CSV rows."""
    with open(file_path, "r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        return []
    header = {str(column).strip().casefold() for column in rows[0].keys()}
    missing = [column for column in _RIVER_REQUIRED_COLUMNS if column.casefold() not in header]
    if missing:
        raise AppError("River CSV is missing required columns: " + ", ".join(missing))
    normalized = []
    for row in rows:
        record = normalize_river_record(row)
        if record is not None:
            normalized.append(record)
    return normalized


def is_river_format(input_format):
    return input_format == "river_csv"


# -- BIP329 ------------------------------------------------------------------


def normalize_bip329_record(record):
    """Validate and normalize a single BIP329 label record.

    Enforces the known `record_type` set, non-empty `ref`, and the
    `spendable`-is-`output`-only rule from the spec.
    """
    if not isinstance(record, dict):
        raise AppError("BIP329 records must be JSON objects")
    record_type = str(record.get("type") or "").strip()
    ref = str(record.get("ref") or "").strip()
    if record_type not in {"tx", "addr", "pubkey", "input", "output", "xpub"}:
        raise AppError(f"Unsupported BIP329 record type '{record_type}'")
    if not ref:
        raise AppError("BIP329 records require a non-empty ref")
    spendable = record.get("spendable")
    if spendable is not None and not isinstance(spendable, bool):
        raise AppError("BIP329 spendable must be a boolean when present")
    if spendable is not None and record_type != "output":
        raise AppError("BIP329 spendable is only valid for output records")
    return {
        "type": record_type,
        "ref": ref,
        "label": str_or_none(record.get("label")),
        "origin": str_or_none(record.get("origin")),
        "spendable": spendable,
        "data": {
            key: value
            for key, value in record.items()
            if key not in {"type", "ref", "label", "origin", "spendable"}
        },
    }


def load_bip329_file(file_path):
    """Load a BIP329 JSONL file (one JSON object per line) and normalize each row."""
    records = []
    with open(file_path, "r", encoding="utf-8") as handle:
        for line_number, raw_line in enumerate(handle, start=1):
            line = raw_line.strip()
            if not line:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError as exc:
                raise AppError(f"Invalid BIP329 JSON on line {line_number}") from exc
            records.append(normalize_bip329_record(payload))
    return records
