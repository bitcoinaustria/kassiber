"""Decimal and msat/sat/BTC conversion helpers.

Kassiber stores BTC-denominated amounts as INTEGER msat on disk — the
Lightning convention where 1 BTC = 100_000_000_000 msat — while Python-side
accounting math still flows as `Decimal` BTC so the engine stays expressive.

`btc_to_msat` / `msat_to_btc` are the adapter boundary: every DB write of an
amount / fee / quantity goes through `btc_to_msat`, every DB read of the
same goes through `msat_to_btc`. Nothing else in the code should touch the
BTC↔msat conversion directly.

`dec` is the single entry point for turning arbitrary-typed (`None`, `""`,
int, float, str, `Decimal`) user/wallet input into a `Decimal`, raising a
structured `AppError` on junk. Call sites should never use the raw
`Decimal()` constructor because it doesn't catch `None` or empty strings.
"""

from decimal import Decimal, InvalidOperation, ROUND_HALF_UP

from .errors import AppError


SATS_PER_BTC = Decimal("100000000")
MSAT_PER_BTC = Decimal("100000000000")


def dec(value, default="0"):
    """Coerce mixed-typed input to `Decimal`, defaulting empties.

    Raises `AppError` on unparseable junk so bad import rows surface a
    validation envelope rather than a bare `InvalidOperation`.
    """
    if value is None:
        return Decimal(default)
    if isinstance(value, str) and value == "":
        return Decimal(default)
    try:
        return Decimal(str(value))
    except InvalidOperation as exc:
        raise AppError(f"Invalid decimal value: {value}") from exc


def btc_to_msat(value):
    """Accept Decimal/float/str BTC and return integer msat (None -> None)."""
    if value is None:
        return None
    amount = dec(value) * MSAT_PER_BTC
    return int(amount.to_integral_value(rounding=ROUND_HALF_UP))


def msat_to_btc(value):
    """Accept integer msat (or any DB row value) and return Decimal BTC (None -> None)."""
    if value is None:
        return None
    return (Decimal(int(value)) / MSAT_PER_BTC)
