"""Decimal validation shared by custody admission and finalized projections."""

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
    return number
