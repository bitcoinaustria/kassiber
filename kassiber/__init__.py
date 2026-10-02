import decimal as _decimal

__all__ = ["__version__"]

__version__ = "0.22.78"

# Tax values are computed at 32 significant digits with round-half-even, as
# RP2 and the native tax engine compute them (docs/reference/tax-engine.md).
# RP2 sets that precision only on the thread that first imports it, so the
# same book was stored with different values depending on which thread
# processed it. Pin it for every thread instead: new threads start from
# DefaultContext, and the importing thread is set here. Traps are unchanged.
_decimal.DefaultContext.prec = 32
_decimal.DefaultContext.rounding = _decimal.ROUND_HALF_EVEN
_decimal.getcontext().prec = 32
_decimal.getcontext().rounding = _decimal.ROUND_HALF_EVEN
