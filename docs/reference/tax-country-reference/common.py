"""Shared helpers for the tax-country reference calculators (research only).

Exact arithmetic uses Fraction; amounts are rounded only when rendered.
"""

from __future__ import annotations

import calendar
from datetime import date, datetime
from decimal import ROUND_HALF_EVEN, Decimal
from fractions import Fraction
from zoneinfo import ZoneInfo


def F(value) -> Fraction:
    if value is None:
        return Fraction(0)
    if isinstance(value, Fraction):
        return value
    return Fraction(Decimal(str(value)))


def money(value: Fraction, places: int = 2) -> str:
    """Render a fiat amount half-even at `places` decimals."""
    q = Decimal(1).scaleb(-places)
    d = Decimal(value.numerator) / Decimal(value.denominator)
    return str(d.quantize(q, rounding=ROUND_HALF_EVEN))


def qty(value: Fraction) -> str:
    """Render an exact quantity (BTC has at most 11 decimals as msat)."""
    d = Decimal(value.numerator) / Decimal(value.denominator)
    d = d.quantize(Decimal("1E-11"), rounding=ROUND_HALF_EVEN).normalize()
    text = format(d, "f")
    return text


def local_dt(text: str, tz: str) -> datetime:
    dt = datetime.fromisoformat(text)
    if dt.tzinfo is None:
        raise ValueError(f"timestamp without offset: {text}")
    return dt.astimezone(ZoneInfo(tz))


def local_date(text: str, tz: str) -> date:
    return local_dt(text, tz).date()


def anniversary(d: date, years: int = 1) -> date:
    """Same calendar date `years` later; 29 Feb maps to 28 Feb (§188 Abs. 3 BGB)."""
    y = d.year + years
    day = min(d.day, calendar.monthrange(y, d.month)[1])
    return date(y, d.month, day)


def add_months(d: date, months: int) -> date:
    m = d.month - 1 + months
    y = d.year + m // 12
    m = m % 12 + 1
    day = min(d.day, calendar.monthrange(y, m)[1])
    return date(y, m, day)
