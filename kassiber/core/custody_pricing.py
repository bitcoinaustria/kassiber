"""Bounded decimal pricing shared by write boundaries and tax projections."""

import math
from decimal import Decimal, InvalidOperation

from ..errors import AppError

# The tax engine (like its RP2 oracle) quantizes comparisons to 13 decimal
# places with 32 significant digits: at most 19 integer digits fit.
TAX_DECIMAL_LIMIT = Decimal("1e19")


def finite_decimal(value, field: str) -> Decimal:
    """Require a decimal that is finite in exact and legacy numeric outputs."""
    try:
        number = Decimal(str(value))
    except (InvalidOperation, ValueError, TypeError) as exc:
        raise AppError(
            f"{field} must be a decimal number", code="validation", retryable=False
        ) from exc
    if not number.is_finite() or not math.isfinite(float(number)):
        raise AppError(
            f"{field} must be a finite decimal number",
            code="validation", retryable=False,
        )
    # Bound before any fixed-point formatting; RP2 compares prices
    # at 13 decimal places. Never admit a nonzero price that becomes zero.
    if number and (
        abs(number.as_tuple().exponent) > 308
        or number.copy_abs() < Decimal("1e-13")
        or number.copy_abs() >= TAX_DECIMAL_LIMIT
        # Legacy numeric mirrors must not round up across the engine limit.
        or abs(float(number)) >= float(TAX_DECIMAL_LIMIT)
    ):
        raise AppError(
            f"{field} is outside the supported decimal range",
            code="validation", retryable=False,
        )
    return number if number else Decimal(0)


def validate_transaction_price(row) -> None:
    """Validate stored fields and both valuations without persisting inference."""
    transaction_id = row["id"]
    def field_value(field):
        return row[field] if field in row.keys() else None
    for field in ("fiat_value", "fiat_value_exact", "fiat_rate", "fiat_rate_exact"):
        if field_value(field) not in (None, ""):
            finite_decimal(field_value(field), f"transaction {transaction_id}: {field}")
    rate = field_value("fiat_rate_exact") or field_value("fiat_rate")
    value = field_value("fiat_value_exact") or field_value("fiat_value")
    from ..msat import msat_to_btc
    amount = abs(msat_to_btc(field_value("amount") or 0))
    if rate not in (None, "") and amount:
        finite_decimal(finite_decimal(rate, "fiat_rate") * amount, f"transaction {transaction_id}: derived fiat_value")
    if value not in (None, "") and amount:
        finite_decimal(finite_decimal(value, "fiat_value") / amount, f"transaction {transaction_id}: derived fiat_rate")
