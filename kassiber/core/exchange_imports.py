from __future__ import annotations

"""Native exchange API import helpers.

The fetchers are deliberately thin: they authenticate, page provider JSON, and
then hand plain payloads to pure normalizers that return Kassiber import-record
dicts. That keeps live credentials out of parser tests and avoids a runtime
dependency on DALI/CCXT.
"""

import base64
import hashlib
import hmac
import json
import time
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any, Mapping
from urllib import parse as urlparse
from urllib import request as urlrequest

from ..asset_codes import (
    BTC_ASSET_ALIASES,
    FIAT_CURRENCIES as NORMALIZED_FIAT_CURRENCIES,
    LBTC_ASSET_ALIASES,
)
from ..backends import backend_timeout, backend_value
from ..errors import AppError
from ..http_client import request_with_retry
from ..msat import dec
from ..proxy import build_proxy_opener
from ..util import str_or_none
from ..wallet_descriptors import normalize_asset_code
from . import pricing

KRAKEN_DEFAULT_URL = "https://api.kraken.com"
COINBASE_DEFAULT_URL = "https://api.coinbase.com"
BINANCE_DEFAULT_URL = "https://api.binance.com"
BTC_ASSETS = BTC_ASSET_ALIASES
LBTC_ASSETS = LBTC_ASSET_ALIASES
KRAKEN_FIAT_CURRENCIES = NORMALIZED_FIAT_CURRENCIES | {
    f"Z{currency}" for currency in NORMALIZED_FIAT_CURRENCIES if len(currency) == 3
}


def _json_ready(value: Any) -> Any:
    if isinstance(value, Decimal):
        return format(value, "f")
    if isinstance(value, Mapping):
        return {str(key): _json_ready(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_json_ready(item) for item in value]
    return value


def _decimal(value: Any, default: str = "0") -> Decimal:
    if value in (None, ""):
        value = default
    return dec(value)


def _asset(value: Any) -> str:
    text = normalize_asset_code(value)
    if text in BTC_ASSETS:
        return "BTC"
    if text in LBTC_ASSETS:
        return "LBTC"
    if text.startswith("Z") and len(text) == 4:
        return text[1:]
    if text.startswith("X") and len(text) == 4 and text[1:] in {"ETH"}:
        return text[1:]
    return text


def _is_btc_asset(value: Any) -> bool:
    return _asset(value) in BTC_ASSETS | {"LBTC"}


def _is_fiat(value: Any) -> bool:
    return _asset(value) in NORMALIZED_FIAT_CURRENCIES


# Legacy-holdings overlay (opt-in ``include_legacy``): non-Bitcoin exchange
# rows pass through to the same record shape instead of being dropped. They
# stay overview-only downstream — the tax engine excludes every non-Bitcoin
# asset (see asset_codes.is_tax_engine_asset). Rows whose quantities do not
# fit the msat-scaled integer storage (≤11 decimals, ~92.2M units) are skipped
# and reported via ``legacy_notes`` — never silently rounded.
_LEGACY_SCALE = Decimal("100000000000")
_LEGACY_MAX_SCALED = Decimal(2**63 - 1)


def _legacy_amount_ok(*values: Decimal | None) -> bool:
    for value in values:
        if value is None:
            continue
        scaled = abs(value) * _LEGACY_SCALE
        if scaled != scaled.to_integral_value() or scaled > _LEGACY_MAX_SCALED:
            return False
    return True


def _note_legacy_skip(legacy_notes, *, provider, external_id, asset, reason):
    if legacy_notes is not None:
        legacy_notes.append(
            {
                "provider": provider,
                "external_id": external_id,
                "asset": asset,
                "reason": reason,
            }
        )


def _ms_epoch_to_iso(value: Any) -> str | None:
    if value in (None, ""):
        return None
    seconds = _decimal(value) / Decimal("1000")
    return (
        datetime.fromtimestamp(float(seconds), tz=timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def _seconds_epoch_to_iso(value: Any) -> str | None:
    if value in (None, ""):
        return None
    return (
        datetime.fromtimestamp(float(_decimal(value)), tz=timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def _record(
    *,
    provider: str,
    external_id: str,
    occurred_at: str | None,
    direction: str,
    asset: str,
    amount: Decimal,
    fee: Decimal = Decimal("0"),
    kind: str,
    raw: Mapping[str, Any],
    description: str | None = None,
    fiat_currency: str | None = None,
    fiat_value: Decimal | None = None,
    fiat_rate: Decimal | None = None,
    pricing_method: str | None = None,
    pricing_external_ref: str | None = None,
) -> dict[str, Any]:
    pricing_source_kind = (
        pricing.SOURCE_EXCHANGE_EXECUTION if fiat_value is not None else None
    )
    if fiat_rate is None and fiat_value is not None and amount:
        fiat_rate = fiat_value / amount
    return {
        "txid": external_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": abs(amount),
        "fee": abs(fee),
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "fiat_currency": fiat_currency,
        "pricing_source_kind": pricing_source_kind,
        "pricing_provider": provider if pricing_source_kind else None,
        "pricing_pair": (
            f"{asset}-{fiat_currency}" if pricing_source_kind and fiat_currency else None
        ),
        "pricing_timestamp": occurred_at if pricing_source_kind else None,
        "pricing_method": pricing_method if pricing_source_kind else None,
        "pricing_external_ref": pricing_external_ref if pricing_source_kind else None,
        "pricing_quality": pricing.QUALITY_EXACT if pricing_source_kind else None,
        "kind": kind,
        "description": description or f"{provider} {kind}",
        "counterparty": provider,
        "raw_json": json.dumps(_json_ready(raw), sort_keys=True),
    }


def _mapping_rows(payload: Any, *keys: str) -> list[tuple[str, Mapping[str, Any]]]:
    value = payload
    for key in keys:
        if isinstance(value, Mapping) and key in value:
            value = value[key]
    if isinstance(value, Mapping):
        return [
            (str(key), item)
            for key, item in value.items()
            if isinstance(item, Mapping)
        ]
    if isinstance(value, list):
        rows = []
        for index, item in enumerate(value, start=1):
            if not isinstance(item, Mapping):
                continue
            row_id = str(item.get("id") or item.get("txid") or item.get("ledger_id") or index)
            rows.append((row_id, item))
        return rows
    return []


def _trade_rows(payload: Any) -> dict[str, Mapping[str, Any]]:
    rows = _mapping_rows(payload, "result", "trades") or _mapping_rows(payload, "trades")
    return {row_id: row for row_id, row in rows}


def normalize_kraken_records(
    ledger_payload: Any,
    trades_payload: Any | None = None,
    *,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    """Normalize Kraken private Ledgers/TradesHistory payloads.

    ``include_legacy`` additionally passes non-Bitcoin (overlay) asset rows
    through; without it they are skipped as before. Overlay rows the record
    shape cannot represent are skipped with a ``legacy_notes`` entry instead
    of failing the sync.
    """
    trades = _trade_rows(trades_payload or {})
    rows = _mapping_rows(ledger_payload, "result", "ledger") or _mapping_rows(
        ledger_payload,
        "ledger",
    )
    records: list[dict[str, Any]] = []
    for ledger_id, row in rows:
        row_asset = _asset(row.get("asset"))
        is_legacy_row = row_asset not in {"BTC", "LBTC"}
        if is_legacy_row and (not include_legacy or _is_fiat(row.get("asset"))):
            continue
        row_type = str(row.get("type") or "").strip().lower()
        amount = _decimal(row.get("amount"))
        fee = _decimal(row.get("fee"))
        occurred_at = _seconds_epoch_to_iso(row.get("time")) or str_or_none(row.get("time"))
        external_id = f"kraken:{ledger_id}"
        raw = {"ledger_id": ledger_id, **dict(row)}
        if is_legacy_row and not _legacy_amount_ok(amount, fee):
            _note_legacy_skip(
                legacy_notes,
                provider="Kraken",
                external_id=external_id,
                asset=row_asset,
                reason="amount outside storable envelope (>11 decimals or >92.2M units)",
            )
            continue
        if is_legacy_row and row_type not in {"deposit", "withdrawal", "trade"}:
            # Overlay assets tolerate exotic ledger types (staking, transfer,
            # dust sweeps) by skipping with a note; Bitcoin rows keep the hard
            # error below so tax-relevant history is never silently dropped.
            _note_legacy_skip(
                legacy_notes,
                provider="Kraken",
                external_id=external_id,
                asset=row_asset,
                reason=f"unsupported ledger type '{row_type}'",
            )
            continue
        if row_type in {"deposit", "withdrawal"}:
            records.append(
                _record(
                    provider="Kraken",
                    external_id=external_id,
                    occurred_at=occurred_at,
                    direction="inbound" if amount >= 0 else "outbound",
                    asset=row_asset,
                    amount=amount,
                    fee=fee,
                    kind="deposit" if amount >= 0 else "withdrawal",
                    raw=raw,
                    pricing_method="kraken_api",
                )
            )
            continue
        if row_type != "trade":
            raise AppError(
                f"Kraken BTC ledger row '{ledger_id}' has unsupported type '{row_type}'",
                code="validation",
                hint=(
                    "Only BTC/LBTC deposits, withdrawals, and fiat-quoted "
                    "trades are imported automatically."
                ),
                retryable=False,
            )
        trade = trades.get(str(row.get("refid") or ""))
        if not trade:
            if is_legacy_row:
                _note_legacy_skip(
                    legacy_notes,
                    provider="Kraken",
                    external_id=external_id,
                    asset=row_asset,
                    reason="trade ledger row without a TradesHistory row",
                )
                continue
            raise AppError(
                f"Kraken trade ledger row '{ledger_id}' is missing its TradesHistory row",
                code="validation",
                hint=(
                    "Provide both Kraken ledger and trade history so execution "
                    "prices can be verified."
                ),
                retryable=False,
            )
        pair = str(trade.get("pair") or "")
        quote = _kraken_quote_from_pair(pair, row_asset)
        if not quote or not _is_fiat(quote):
            if include_legacy:
                # Crypto-quoted (cross-asset) trade: emit the leg unpriced.
                # Bitcoin legs are priced later from cached market rates at
                # journal time; overlay legs stay overview-only anyway.
                direction = "inbound" if amount >= 0 else "outbound"
                records.append(
                    _record(
                        provider="Kraken",
                        external_id=external_id,
                        occurred_at=occurred_at,
                        direction=direction,
                        asset=row_asset,
                        amount=amount,
                        fee=fee if not _is_fiat(row.get("asset")) else Decimal("0"),
                        kind="buy" if direction == "inbound" else "sell",
                        raw={**raw, "trade": dict(trade)},
                        pricing_external_ref=str(row.get("refid") or ledger_id),
                    )
                )
                continue
            raise AppError(
                f"Kraken BTC trade '{ledger_id}' is not fiat-quoted",
                code="validation",
                hint=(
                    "Cross-asset Kraken trades need explicit user review "
                    "through the generic ledger, or re-run with "
                    "--include-legacy-assets to import both legs unpriced."
                ),
                retryable=False,
            )
        fiat_currency = _asset(quote)
        cost = abs(_decimal(trade.get("cost")))
        trade_fee = abs(_decimal(trade.get("fee")))
        direction = "inbound" if amount >= 0 else "outbound"
        fiat_value = cost if direction == "inbound" else max(Decimal("0"), cost - trade_fee)
        records.append(
            _record(
                provider="Kraken",
                external_id=external_id,
                occurred_at=occurred_at,
                direction=direction,
                asset=row_asset,
                amount=amount,
                fee=fee if not _is_fiat(row.get("asset")) else Decimal("0"),
                kind="buy" if direction == "inbound" else "sell",
                raw={**raw, "trade": dict(trade)},
                fiat_currency=fiat_currency,
                fiat_value=fiat_value,
                fiat_rate=(
                    _decimal(trade.get("price"))
                    if trade.get("price") not in (None, "")
                    else None
                ),
                pricing_method="kraken_api",
                pricing_external_ref=str(row.get("refid") or ledger_id),
            )
        )
    return records


def _kraken_quote_from_pair(pair: str, base_asset: str) -> str | None:
    normalized = pair.upper().replace("/", "")
    base_candidates = [base_asset, "XBT" if base_asset == "BTC" else base_asset]
    for base in base_candidates:
        for prefix in (base, f"X{base}"):
            if normalized.startswith(prefix):
                return normalized[len(prefix) :]
    for quote in sorted(KRAKEN_FIAT_CURRENCIES, key=len, reverse=True):
        if normalized.endswith(quote):
            return quote
    return None


COINBASE_TRADE_TYPES = {"buy", "sell", "trade", "advanced_trade_fill"}
COINBASE_MOVEMENT_TYPES = {
    "send",
    "exchange_deposit",
    "exchange_withdrawal",
    "pro_deposit",
    "pro_withdrawal",
    "prime_withdrawal",
}
COINBASE_INCOME_TYPES = {"interest", "staking_reward", "inflation_reward"}


def normalize_coinbase_records(
    payload: Any,
    *,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    """Normalize Coinbase API v2 account/transaction payloads.

    ``include_legacy`` additionally passes non-Bitcoin (overlay) asset rows
    through; unrepresentable overlay rows are skipped with a ``legacy_notes``
    entry instead of failing the sync.
    """
    transactions: list[tuple[str, Mapping[str, Any]]] = []
    if isinstance(payload, Mapping) and isinstance(payload.get("transactions"), list):
        currency = str(payload.get("currency") or "")
        transactions.extend(
            (currency, tx)
            for tx in payload["transactions"]
            if isinstance(tx, Mapping)
        )
    elif isinstance(payload, list):
        for item in payload:
            if not isinstance(item, Mapping):
                continue
            currency = str(item.get("currency") or item.get("account_currency") or "")
            if "transactions" in item and isinstance(item["transactions"], list):
                transactions.extend(
                    (currency, tx) for tx in item["transactions"] if isinstance(tx, Mapping)
                )
            else:
                transactions.append((currency, item))
    records = []
    for account_currency, tx in transactions:
        currency = _coinbase_currency(account_currency, tx)
        is_legacy_row = not _is_btc_asset(currency)
        if is_legacy_row and (not include_legacy or not currency or _is_fiat(currency)):
            continue
        record = normalize_coinbase_transaction(
            tx, currency, legacy=is_legacy_row, legacy_notes=legacy_notes
        )
        if record is None:
            continue
        if is_legacy_row and not _legacy_amount_ok(record.get("amount"), record.get("fee")):
            _note_legacy_skip(
                legacy_notes,
                provider="Coinbase",
                external_id=str(record.get("txid") or ""),
                asset=currency,
                reason="amount outside storable envelope (>11 decimals or >92.2M units)",
            )
            continue
        records.append(record)
    return records


def _coinbase_currency(account_currency: str, tx: Mapping[str, Any]) -> str:
    amount = tx.get("amount") if isinstance(tx.get("amount"), Mapping) else {}
    currency = amount.get("currency") if isinstance(amount, Mapping) else None
    return _asset(currency or account_currency)


def _coinbase_money(tx: Mapping[str, Any], key: str) -> tuple[Decimal, str | None]:
    value = tx.get(key)
    if not isinstance(value, Mapping):
        return Decimal("0"), None
    return (
        _decimal(value.get("amount")),
        _asset(value.get("currency")) if value.get("currency") else None,
    )


def normalize_coinbase_transaction(
    tx: Mapping[str, Any],
    currency: str,
    *,
    legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> dict[str, Any] | None:
    tx_type = str(tx.get("type") or "").strip().lower()
    amount, _amount_currency = _coinbase_money(tx, "amount")
    if amount == 0:
        return None
    native_amount, native_currency = _coinbase_money(tx, "native_amount")
    occurred_at = str_or_none(tx.get("created_at") or tx.get("createdAt"))
    tx_id = str(tx.get("id") or "")
    external_id = f"coinbase:{tx_id}" if tx_id else f"coinbase:{occurred_at}:{currency}:{amount}"
    raw = dict(tx)
    if tx_type in COINBASE_TRADE_TYPES:
        if (
            native_currency is None
            or not _is_fiat(native_currency)
            or abs(native_amount) < Decimal("0.01")
        ):
            if legacy:
                # Overlay trade without a usable fiat execution value: keep
                # the leg unpriced rather than failing the whole sync.
                direction = "inbound" if amount > 0 else "outbound"
                return _record(
                    provider="Coinbase",
                    external_id=external_id,
                    occurred_at=occurred_at,
                    direction=direction,
                    asset=currency,
                    amount=amount,
                    kind="buy" if direction == "inbound" else "sell",
                    raw=raw,
                    pricing_external_ref=tx_id,
                )
            raise AppError(
                f"Coinbase BTC trade '{tx_id}' does not include usable fiat execution value",
                code="validation",
                retryable=False,
            )
        direction = "inbound" if amount > 0 else "outbound"
        detail = tx.get(tx_type) if isinstance(tx.get(tx_type), Mapping) else {}
        commission = (
            _decimal(detail.get("commission"), "0")
            if isinstance(detail, Mapping)
            else Decimal("0")
        )
        fiat_value = abs(native_amount)
        if direction == "inbound" and commission:
            fiat_value += abs(commission)
        return _record(
            provider="Coinbase",
            external_id=external_id,
            occurred_at=occurred_at,
            direction=direction,
            asset=currency,
            amount=amount,
            kind="buy" if direction == "inbound" else "sell",
            raw=raw,
            fiat_currency=native_currency,
            fiat_value=fiat_value,
            pricing_method="coinbase_api",
            pricing_external_ref=tx_id,
        )
    if tx_type in COINBASE_INCOME_TYPES:
        return _record(
            provider="Coinbase",
            external_id=external_id,
            occurred_at=occurred_at,
            direction="inbound",
            asset=currency,
            amount=amount,
            kind="interest" if tx_type == "interest" else "staking",
            raw=raw,
            description=f"Coinbase {tx_type.replace('_', ' ')}",
        )
    if tx_type in COINBASE_MOVEMENT_TYPES:
        network = tx.get("network") if isinstance(tx.get("network"), Mapping) else {}
        tx_hash = str_or_none(network.get("hash")) if isinstance(network, Mapping) else None
        return _record(
            provider="Coinbase",
            external_id=tx_hash or external_id,
            occurred_at=occurred_at,
            direction="inbound" if amount > 0 else "outbound",
            asset=currency,
            amount=amount,
            kind="deposit" if amount > 0 else "withdrawal",
            raw=raw,
            description=f"Coinbase {tx_type.replace('_', ' ')}",
        )
    if legacy:
        _note_legacy_skip(
            legacy_notes,
            provider="Coinbase",
            external_id=external_id,
            asset=currency,
            reason=f"unsupported transaction type '{tx_type}'",
        )
        return None
    raise AppError(
        f"Coinbase BTC transaction '{tx_id}' has unsupported type '{tx_type}'",
        code="validation",
        hint=(
            "Unsupported Coinbase BTC rows are not guessed; export them "
            "through the generic ledger with explicit semantics."
        ),
        retryable=False,
    )


def normalize_binance_records(
    payload: Mapping[str, Any],
    *,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []

    def _append(record: dict[str, Any] | None) -> None:
        if record is None:
            return
        asset = str(record.get("asset") or "")
        if asset not in {"BTC", "LBTC"} and not _legacy_amount_ok(
            record.get("amount"), record.get("fee")
        ):
            _note_legacy_skip(
                legacy_notes,
                provider="Binance",
                external_id=str(record.get("txid") or ""),
                asset=asset,
                reason="amount outside storable envelope (>11 decimals or >92.2M units)",
            )
            return
        records.append(record)

    for row in payload.get("fiat_payments", []) or []:
        if not isinstance(row, Mapping):
            continue
        _append(normalize_binance_fiat_payment(row, include_legacy=include_legacy))
    for row in payload.get("deposits", []) or []:
        if not isinstance(row, Mapping):
            continue
        _append(normalize_binance_transfer(row, inbound=True, include_legacy=include_legacy))
    for row in payload.get("withdrawals", []) or []:
        if not isinstance(row, Mapping):
            continue
        _append(normalize_binance_transfer(row, inbound=False, include_legacy=include_legacy))
    for row in payload.get("dividends", []) or []:
        if not isinstance(row, Mapping):
            continue
        _append(normalize_binance_dividend(row, include_legacy=include_legacy))
    return records


def normalize_binance_fiat_payment(
    row: Mapping[str, Any], *, include_legacy: bool = False
) -> dict[str, Any] | None:
    if str(row.get("status") or "").strip().casefold() != "completed":
        return None
    asset = _asset(row.get("cryptoCurrency"))
    if asset not in {"BTC", "LBTC"} and not (include_legacy and asset and not _is_fiat(asset)):
        return None
    fiat_currency = _asset(row.get("fiatCurrency"))
    if not _is_fiat(fiat_currency):
        raise AppError("Binance fiat payment is not fiat-denominated", code="validation")
    amount = _decimal(row.get("obtainAmount"))
    fiat_value = abs(_decimal(row.get("sourceAmount")))
    occurred_at = _ms_epoch_to_iso(row.get("createTime"))
    ref = str(row.get("orderNo") or row.get("orderId") or occurred_at)
    return _record(
        provider="Binance",
        external_id=f"binance:{ref}",
        occurred_at=occurred_at,
        direction="inbound",
        asset=asset,
        amount=amount,
        kind="buy",
        raw=dict(row),
        fiat_currency=fiat_currency,
        fiat_value=fiat_value,
        fiat_rate=(
            _decimal(row.get("price"))
            if row.get("price") not in (None, "")
            else None
        ),
        pricing_method="binance_api",
        pricing_external_ref=ref,
    )


def normalize_binance_transfer(
    row: Mapping[str, Any], *, inbound: bool, include_legacy: bool = False
) -> dict[str, Any] | None:
    asset = _asset(row.get("coin") or row.get("asset"))
    if asset not in {"BTC", "LBTC"} and not (include_legacy and asset and not _is_fiat(asset)):
        return None
    status = str(row.get("status") or "").strip().casefold()
    if status and status not in {"1", "6", "success", "completed", "credited", "confirming"}:
        return None
    amount = _decimal(row.get("amount"))
    fee = _decimal(row.get("transactionFee") or row.get("fee"), "0")
    occurred_at = _ms_epoch_to_iso(
        row.get("insertTime") or row.get("applyTime") or row.get("successTime")
    )
    ref = str_or_none(row.get("txId") or row.get("id") or row.get("withdrawOrderId"))
    return _record(
        provider="Binance",
        external_id=ref or f"binance:{occurred_at}:{asset}:{amount}",
        occurred_at=occurred_at,
        direction="inbound" if inbound else "outbound",
        asset=asset,
        amount=amount,
        fee=fee if not inbound else Decimal("0"),
        kind="deposit" if inbound else "withdrawal",
        raw=dict(row),
        description="Binance deposit" if inbound else "Binance withdrawal",
    )


def normalize_binance_dividend(
    row: Mapping[str, Any], *, include_legacy: bool = False
) -> dict[str, Any] | None:
    asset = _asset(row.get("asset") or row.get("coinName"))
    if asset not in {"BTC", "LBTC"} and not (include_legacy and asset and not _is_fiat(asset)):
        return None
    amount = _decimal(row.get("amount") or row.get("profitAmount"))
    if amount == 0:
        return None
    ref = str(row.get("id") or row.get("tranId") or row.get("time") or row.get("divTime"))
    occurred_at = _ms_epoch_to_iso(row.get("divTime") or row.get("time"))
    info = str_or_none(row.get("enInfo") or row.get("type"))
    kind = "mining" if info and "mining" in info.casefold() else "income"
    return _record(
        provider="Binance",
        external_id=f"binance:{ref}",
        occurred_at=occurred_at,
        direction="inbound",
        asset=asset,
        amount=amount,
        kind=kind,
        raw=dict(row),
        description="Binance income" + (f" - {info}" if info else ""),
    )


def _backend_opener(backend: Mapping[str, Any], opener=None):
    return opener or build_proxy_opener(backend_value(backend, "tor_proxy"))


def _read_json(opener, request: urlrequest.Request, timeout: int, source_label: str):
    url = request.full_url

    def open_once():
        with opener.open(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))

    return request_with_retry(url, open_once, source_label=source_label)


def fetch_kraken_records(
    backend: Mapping[str, Any],
    *,
    opener=None,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    key = backend_value(backend, "token", "api_key", "username")
    secret = backend_value(backend, "auth_header", "api_secret", "password")
    if not key or not secret:
        raise AppError(
            "Kraken backend is missing API key/secret",
            code="config_error",
            hint="Store the key in token and the secret in auth_header (or password).",
        )
    base = (backend_value(backend, "url") or KRAKEN_DEFAULT_URL).rstrip("/")
    timeout = backend_timeout(backend)
    http = _backend_opener(backend, opener)
    trades = _kraken_private_paginated(
        http,
        base,
        "/0/private/TradesHistory",
        "trades",
        key,
        secret,
        timeout,
    )
    ledgers = _kraken_private_paginated(
        http,
        base,
        "/0/private/Ledgers",
        "ledger",
        key,
        secret,
        timeout,
    )
    return normalize_kraken_records(
        ledgers, trades, include_legacy=include_legacy, legacy_notes=legacy_notes
    )


def _kraken_private_paginated(opener, base, path, result_key, key, secret, timeout):
    rows: dict[str, Mapping[str, Any]] = {}
    offset = 0
    total: int | None = None
    while total is None or offset < total:
        payload = _kraken_private_post(
            opener,
            base,
            path,
            key,
            secret,
            timeout,
            params={"ofs": str(offset)},
        )
        result = payload.get("result") if isinstance(payload, Mapping) else None
        if not isinstance(result, Mapping):
            raise AppError("Kraken response was not a JSON result object", code="protocol_error")
        page = result.get(result_key)
        if not isinstance(page, Mapping):
            raise AppError(
                f"Kraken response result.{result_key} was not a JSON object",
                code="protocol_error",
            )
        rows.update(
            (str(row_id), row)
            for row_id, row in page.items()
            if isinstance(row, Mapping)
        )
        try:
            total = int(result.get("count", len(rows)))
        except (TypeError, ValueError):
            total = len(rows)
        if not page or len(rows) >= total:
            break
        offset += len(page)
    return {"result": {result_key: rows, "count": len(rows)}}


def _kraken_private_post(opener, base, path, key, secret, timeout, *, params=None):
    nonce = str(time.time_ns())
    payload = {"nonce": nonce, **dict(params or {})}
    body = urlparse.urlencode(payload).encode("utf-8")
    sha = hashlib.sha256(nonce.encode("utf-8") + body).digest()
    try:
        decoded_secret = base64.b64decode(secret)
    except Exception as exc:  # noqa: BLE001
        raise AppError("Kraken API secret must be base64 encoded", code="config_error") from exc
    signature = hmac.new(decoded_secret, path.encode("utf-8") + sha, hashlib.sha512)
    request = urlrequest.Request(
        f"{base}{path}",
        data=body,
        headers={
            "API-Key": key,
            "API-Sign": base64.b64encode(signature.digest()).decode("ascii"),
            "Content-Type": "application/x-www-form-urlencoded",
        },
        method="POST",
    )
    payload = _read_json(opener, request, timeout, "Kraken")
    errors = payload.get("error") if isinstance(payload, Mapping) else None
    if errors:
        raise AppError(
            "Kraken API returned an error",
            code=(
                "auth_error"
                if any("permission" in str(item).lower() for item in errors)
                else "protocol_error"
            ),
            details={"kraken_errors": [str(item) for item in errors]},
            retryable=False,
        )
    return payload


def fetch_coinbase_records(
    backend: Mapping[str, Any],
    *,
    opener=None,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    key = backend_value(backend, "token", "api_key", "username")
    secret = backend_value(backend, "auth_header", "api_secret", "password")
    if not key or not secret:
        raise AppError(
            "Coinbase backend is missing API key/secret",
            code="config_error",
            hint="Store the key in token and the secret in auth_header (or password).",
        )
    base = (backend_value(backend, "url") or COINBASE_DEFAULT_URL).rstrip("/")
    timeout = backend_timeout(backend)
    http = _backend_opener(backend, opener)
    accounts = _coinbase_get_paginated(http, base, "/v2/accounts", key, secret, timeout)
    payload = []
    for account in accounts:
        if not isinstance(account, Mapping):
            continue
        currency = account.get("currency")
        if isinstance(currency, Mapping):
            currency_code = currency.get("code")
        else:
            currency_code = account.get("currency")
        if not _is_btc_asset(currency_code):
            # Only pull non-Bitcoin account histories when the caller opted
            # into the legacy overlay; fiat accounts are never transactions.
            if not include_legacy or not currency_code or _is_fiat(currency_code):
                continue
        account_id = str(account.get("id") or "")
        if not account_id:
            continue
        transactions = _coinbase_get_paginated(
            http,
            base,
            f"/v2/accounts/{urlparse.quote(account_id)}/transactions",
            key,
            secret,
            timeout,
        )
        payload.append({"currency": currency_code, "transactions": transactions})
    return normalize_coinbase_records(
        payload, include_legacy=include_legacy, legacy_notes=legacy_notes
    )


def _coinbase_get_paginated(opener, base, path, key, secret, timeout):
    rows = []
    next_path: str | None = path
    while next_path:
        url = f"{base}{next_path}"
        timestamp = str(int(time.time()))
        message = f"{timestamp}GET{next_path}"
        signature = hmac.new(
            secret.encode("utf-8"),
            message.encode("utf-8"),
            hashlib.sha256,
        ).hexdigest()
        request = urlrequest.Request(
            url,
            headers={
                "Accept": "application/json",
                "CB-VERSION": "2017-11-27",
                "CB-ACCESS-KEY": key,
                "CB-ACCESS-SIGN": signature,
                "CB-ACCESS-TIMESTAMP": timestamp,
            },
        )
        payload = _read_json(opener, request, timeout, "Coinbase")
        if not isinstance(payload, Mapping):
            raise AppError("Coinbase response was not a JSON object", code="protocol_error")
        data = payload.get("data")
        if not isinstance(data, list):
            raise AppError("Coinbase response data was not a JSON array", code="protocol_error")
        rows.extend(data)
        pagination = (
            payload.get("pagination")
            if isinstance(payload.get("pagination"), Mapping)
            else {}
        )
        next_uri = pagination.get("next_uri") if isinstance(pagination, Mapping) else None
        next_path = str(next_uri) if next_uri else None
    return rows


def fetch_binance_records(
    backend: Mapping[str, Any],
    *,
    opener=None,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    key = backend_value(backend, "token", "api_key", "username")
    secret = backend_value(backend, "auth_header", "api_secret", "password")
    if not key or not secret:
        raise AppError(
            "Binance backend is missing API key/secret",
            code="config_error",
            hint="Store the key in token and the secret in auth_header (or password).",
        )
    base = (backend_value(backend, "url") or BINANCE_DEFAULT_URL).rstrip("/")
    timeout = backend_timeout(backend)
    http = _backend_opener(backend, opener)
    now_ms = int(time.time() * 1000)
    # Binance.com spot trading launched in July 2017; use a broad fixed lower
    # bound so the first import can see full account history for supported APIs.
    start_ms = 1498867200000
    payload = {
        "fiat_payments": _binance_signed_get(
            http,
            base,
            "/sapi/v1/fiat/payments",
            key,
            secret,
            timeout,
            {
                "transactionType": "0",
                "beginTime": str(start_ms),
                "endTime": str(now_ms),
            },
        ).get("data", []),
        "deposits": _binance_signed_get(
            http,
            base,
            "/sapi/v1/capital/deposit/hisrec",
            key,
            secret,
            timeout,
            {"coin": "BTC", "startTime": str(start_ms), "endTime": str(now_ms)},
        ),
        "withdrawals": _binance_signed_get(
            http,
            base,
            "/sapi/v1/capital/withdraw/history",
            key,
            secret,
            timeout,
            {"coin": "BTC", "startTime": str(start_ms), "endTime": str(now_ms)},
        ),
        "dividends": _binance_signed_get(
            http,
            base,
            "/sapi/v1/asset/assetDividend",
            key,
            secret,
            timeout,
            {
                "asset": "BTC",
                "startTime": str(start_ms),
                "endTime": str(now_ms),
                "limit": "500",
            },
        ).get("rows", []),
    }
    return normalize_binance_records(
        payload, include_legacy=include_legacy, legacy_notes=legacy_notes
    )


def _binance_signed_get(opener, base, path, key, secret, timeout, params):
    params = {key_: value for key_, value in params.items() if value not in (None, "")}
    params["timestamp"] = str(int(time.time() * 1000))
    query = urlparse.urlencode(params)
    signature = hmac.new(
        secret.encode("utf-8"),
        query.encode("utf-8"),
        hashlib.sha256,
    ).hexdigest()
    url = f"{base}{path}?{query}&signature={signature}"
    request = urlrequest.Request(
        url,
        headers={"Accept": "application/json", "X-MBX-APIKEY": key},
    )
    return _read_json(opener, request, timeout, "Binance")


def fetch_exchange_records(
    backend: Mapping[str, Any],
    *,
    opener=None,
    include_legacy: bool = False,
    legacy_notes: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    kind = str(backend_value(backend, "kind") or "").strip().lower()
    if kind == "kraken":
        return fetch_kraken_records(
            backend, opener=opener, include_legacy=include_legacy, legacy_notes=legacy_notes
        )
    if kind == "coinbase":
        return fetch_coinbase_records(
            backend, opener=opener, include_legacy=include_legacy, legacy_notes=legacy_notes
        )
    if kind == "binance":
        return fetch_binance_records(
            backend, opener=opener, include_legacy=include_legacy, legacy_notes=legacy_notes
        )
    raise AppError(
        f"Backend kind '{kind}' is not an exchange API importer",
        code="validation",
        hint="Use a backend kind of kraken, coinbase, or binance.",
        retryable=False,
    )


__all__ = [
    "fetch_binance_records",
    "fetch_coinbase_records",
    "fetch_exchange_records",
    "fetch_kraken_records",
    "normalize_binance_records",
    "normalize_coinbase_records",
    "normalize_kraken_records",
]
