"""Bounded decimal pricing shared by write boundaries and tax projections."""

import math
from decimal import Decimal, InvalidOperation

from ..errors import AppError


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
        abs(number.as_tuple().exponent) > 308 or abs(number) < Decimal("1e-13")
    ):
        raise AppError(
            f"{field} is outside the supported decimal range",
            code="validation", retryable=False,
        )
    return number if number else Decimal(0)
