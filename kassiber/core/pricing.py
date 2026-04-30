from __future__ import annotations

"""Pricing provenance helpers.

Kassiber keeps the legacy REAL fiat columns for compatibility, but exact
accounting values and review decisions should use these typed provenance
fields wherever possible.
"""

from decimal import Decimal
from typing import Any, Mapping

from ..msat import dec

LEGACY_SOURCE_IMPORT = "import"
LEGACY_SOURCE_MANUAL = "manual"
LEGACY_SOURCE_RATES_CACHE = "rates_cache"

SOURCE_GENERIC_IMPORT = "generic_import"
SOURCE_WALLET_EXPORT = "wallet_export"
SOURCE_EXCHANGE_EXECUTION = "exchange_execution"
SOURCE_BTCPAY_WALLET_EXPORT = "btcpay_wallet_export"
SOURCE_BTCPAY_INVOICE = "btcpay_invoice"
SOURCE_BTCPAY_PAYMENT = "btcpay_payment"
SOURCE_MANUAL_OVERRIDE = "manual_override"
SOURCE_MANUAL_RATE_CACHE = "manual_rate_cache"
SOURCE_FMV_PROVIDER = "fmv_provider"

QUALITY_EXACT = "exact"
QUALITY_PROVIDER_SAMPLE = "provider_sample"
QUALITY_COARSE_FALLBACK = "coarse_fallback"
QUALITY_MISSING = "missing"

SOURCE_PRIORITY = {
    SOURCE_FMV_PROVIDER: 10,
    SOURCE_MANUAL_RATE_CACHE: 20,
    SOURCE_GENERIC_IMPORT: 30,
    SOURCE_WALLET_EXPORT: 40,
    SOURCE_BTCPAY_WALLET_EXPORT: 45,
    SOURCE_EXCHANGE_EXECUTION: 70,
    SOURCE_BTCPAY_INVOICE: 80,
    SOURCE_BTCPAY_PAYMENT: 80,
    SOURCE_MANUAL_OVERRIDE: 100,
}

COARSE_GRANULARITIES = {"daily", "coarse", "monthly", "yearly"}


def exact_decimal(value: Any) -> str | None:
    if value in (None, ""):
        return None
    return format(dec(value), "f")


def decimal_from_exact(*values: Any) -> Decimal | None:
    for value in values:
        if value not in (None, ""):
            return dec(value)
    return None


def legacy_source_for(source_kind: str | None) -> str | None:
    if source_kind in {SOURCE_FMV_PROVIDER, SOURCE_MANUAL_RATE_CACHE}:
        return LEGACY_SOURCE_RATES_CACHE
    if source_kind == SOURCE_MANUAL_OVERRIDE:
        return LEGACY_SOURCE_MANUAL
    if source_kind:
        return LEGACY_SOURCE_IMPORT
    return None


def priority_for(source_kind: str | None, legacy_source: str | None = None) -> int:
    if source_kind in SOURCE_PRIORITY:
        return SOURCE_PRIORITY[source_kind]
    if legacy_source == LEGACY_SOURCE_MANUAL:
        return SOURCE_PRIORITY[SOURCE_MANUAL_OVERRIDE]
    if legacy_source == LEGACY_SOURCE_RATES_CACHE:
        return SOURCE_PRIORITY[SOURCE_FMV_PROVIDER]
    if legacy_source == LEGACY_SOURCE_IMPORT:
        return SOURCE_PRIORITY[SOURCE_GENERIC_IMPORT]
    return 0


def infer_import_source_kind(source_label: str, record: Mapping[str, Any]) -> str:
    explicit = record.get("pricing_source_kind") or record.get("fiat_price_source_kind")
    if explicit:
        return str(explicit).strip().lower()
    label = str(source_label or "").strip().lower()
    if label.startswith("btcpay"):
        return SOURCE_BTCPAY_WALLET_EXPORT
    if label.startswith("phoenix"):
        return SOURCE_WALLET_EXPORT
    return SOURCE_GENERIC_IMPORT


def import_quality(source_kind: str) -> str:
    if source_kind in {
        SOURCE_EXCHANGE_EXECUTION,
        SOURCE_BTCPAY_INVOICE,
        SOURCE_BTCPAY_PAYMENT,
        SOURCE_MANUAL_OVERRIDE,
    }:
        return QUALITY_EXACT
    return QUALITY_PROVIDER_SAMPLE if source_kind == SOURCE_GENERIC_IMPORT else QUALITY_EXACT


def rate_cache_source_kind(rate_row: Mapping[str, Any]) -> str:
    return (
        SOURCE_MANUAL_RATE_CACHE
        if str(rate_row.get("source") or "").strip().lower() == "manual"
        else SOURCE_FMV_PROVIDER
    )


def rate_cache_quality(rate_row: Mapping[str, Any]) -> str:
    source_kind = rate_cache_source_kind(rate_row)
    if source_kind == SOURCE_MANUAL_RATE_CACHE:
        return QUALITY_EXACT
    granularity = str(rate_row.get("granularity") or "").strip().lower()
    if granularity in COARSE_GRANULARITIES:
        return QUALITY_COARSE_FALLBACK
    return QUALITY_PROVIDER_SAMPLE


def pricing_payload(
    *,
    rate: Any = None,
    value: Any = None,
    source_kind: str | None,
    quality: str | None,
    provider: str | None = None,
    pair: str | None = None,
    pricing_timestamp: str | None = None,
    fetched_at: str | None = None,
    granularity: str | None = None,
    method: str | None = None,
    external_ref: str | None = None,
) -> dict[str, Any]:
    rate_exact = exact_decimal(rate)
    value_exact = exact_decimal(value)
    return {
        "fiat_rate": float(dec(rate_exact)) if rate_exact is not None else None,
        "fiat_value": float(dec(value_exact)) if value_exact is not None else None,
        "fiat_rate_exact": rate_exact,
        "fiat_value_exact": value_exact,
        "fiat_price_source": legacy_source_for(source_kind),
        "pricing_source_kind": source_kind,
        "pricing_provider": provider,
        "pricing_pair": pair,
        "pricing_timestamp": pricing_timestamp,
        "pricing_fetched_at": fetched_at,
        "pricing_granularity": granularity,
        "pricing_method": method,
        "pricing_external_ref": external_ref,
        "pricing_quality": quality or QUALITY_MISSING,
    }


def journal_exact_payload(entry: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "fiat_value_exact": exact_decimal(entry.get("fiat_value")),
        "unit_cost_exact": exact_decimal(entry.get("unit_cost")),
        "cost_basis_exact": exact_decimal(entry.get("cost_basis")),
        "proceeds_exact": exact_decimal(entry.get("proceeds")),
        "gain_loss_exact": exact_decimal(entry.get("gain_loss")),
    }
