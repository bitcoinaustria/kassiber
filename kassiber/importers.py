"""File-format parsers for wallet-export imports.

Call sites hand a file path + format tag in and get back a list of
normalized record dicts out — one dict per transaction, with the same
shape across all formats so downstream DB-insertion code
(`insert_wallet_records` in `kassiber.core.imports`) doesn't need to
branch per wallet.

Four format families live here:

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


def normalize_phoenix_record(record):
    """Turn a Phoenix CSV row into the common import-record shape.

    Phoenix ships amounts as signed INTEGER msat, fees split across
    `mining_fee_sat` (×1000 to msat) + `service_fee_msat`, and fiat as a
    signed "<value> <CCY>" string. We take the absolute value everywhere
    (direction is captured separately) and derive `fiat_rate` from
    `fiat_value / amount_btc` since Phoenix does not export the rate.

    Private keys `_phoenix_type`, `_phoenix_description`, and
    `_phoenix_onchain_txid` feed `apply_phoenix_metadata` in
    `kassiber.core.imports`.
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
    txid = str_or_none(sanitized.get("tx_id")) or str_or_none(sanitized.get("payment_hash"))
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
        "_phoenix_type": phoenix_type,
        "_phoenix_description": description,
        "_phoenix_onchain_txid": txid,
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


def _casefold_record(record):
    output = {}
    for key, value in record.items():
        if key is None:
            continue
        output[str(key).strip().casefold()] = value
    return output


def _get_cell(record, *names):
    folded = _casefold_record(record)
    for name in names:
        value = folded.get(name.casefold())
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
    for symbol in ("$", "€", "£", "CHF", "USD", "EUR", "GBP", "BTC"):
        text = text.replace(symbol, "")
    text = text.replace(",", "").replace(" ", "").strip()
    if not text:
        return None
    if negative and not text.startswith("-"):
        text = "-" + text
    return text


def _river_decimal(value):
    text = _clean_decimal_text(value)
    return dec(text) if text is not None else None


def _river_currency(value):
    currency = str_or_none(value)
    return normalize_asset_code(currency) if currency else None


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
    return {
        "txid": reference,
        "occurred_at": _get_cell(sanitized, "Date"),
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee_btc,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
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
