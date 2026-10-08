"""The tax engine's errors in Python, and the exceptions its failures raise.

The engine reports every failure with a class and RP2's exact message
(``tax-engine/core/src/error.rs``). Here those classes become Python
exceptions: RP2's own errors keep RP2's messages under Kassiber's names
(``EngineValueError`` for ``RP2ValueError`` and so on, a listed difference of
plan 20), ``decimal`` signals stay ``decimal`` signals, Python's own
``ValueError`` (Kassiber's Austrian time parse) stays one, and a malformed
request or an operation the engine does not implement is an
``EngineContractError``, which blocks the report like a panic.

A ledger failure (schema 2) also names the stage it failed at.
``ledger_error`` raises what the phase-3 adapter raised there: the bare error,
or the adapter's ``AppError`` wrapping it (design §3.9).
"""

from __future__ import annotations

from decimal import DivisionByZero, InvalidOperation, Overflow
from typing import Any, Mapping, Optional

from ...errors import AppError


class EngineError(Exception):
    """An RP2 error the engine reports, with RP2's message."""

    def __init__(self, message: str) -> None:
        self.__message = message
        super().__init__(message)

    def __repr__(self) -> str:
        return self.__message

    @property
    def message(self) -> str:
        return self.__message


class EngineValueError(EngineError):
    """RP2's ``RP2ValueError``."""


class EngineTypeError(EngineError):
    """RP2's ``RP2TypeError``."""


class EngineRuntimeError(EngineError):
    """RP2's ``RP2RuntimeError``."""


class EngineContractError(RuntimeError):
    """A malformed request or an operation the engine does not implement.

    Both are caller or engine bugs, never a property of the book, so they
    block the report like a Rust panic (which the binding raises as
    ``RuntimeError``). ``invariant`` names the shape a finalized projection
    cannot have when the request builder refused the book for one
    (``ledger_request.INVARIANTS``).
    """

    def __init__(self, error_class: str, message: str, *, invariant: Optional[str] = None) -> None:
        self.error_class = error_class
        self.invariant = invariant
        super().__init__(f"{error_class}: {message}")


ERROR_CLASSES: dict[str, type[Exception]] = {
    "ValueError": EngineValueError,
    "TypeError": EngineTypeError,
    "RuntimeError": EngineRuntimeError,
    # RP2's arithmetic raises these ``decimal`` signals unwrapped.
    "InvalidOperation": InvalidOperation,
    "DivisionByZero": DivisionByZero,
    "Overflow": Overflow,
    # Python's own ``ValueError``, which Kassiber's Austrian rules raise for
    # a time ``datetime.fromisoformat`` cannot read
    # (``austrian.infer_regime_from_timestamp``).
    "BuiltinValueError": ValueError,
}


def engine_error(body: Mapping[str, Any]) -> Exception:
    """The exception for an engine error ``{"class", "message"}``."""
    error_class = str(body.get("class"))
    message = str(body.get("message"))
    exception = ERROR_CLASSES.get(error_class)
    if exception is None:
        return EngineContractError(error_class, message)
    return exception(message)


# The stages of a ledger failure and the ``AppError`` the adapter wrapped
# each stage's error in: message prefix and code (``None``: raised bare).
STAGES: dict[str, Optional[tuple[str, str]]] = {
    # Entry constructors, in walk order (engines/rp2.py:912-932, 1109-1128,
    # 1233-1258).
    "prepare": None,
    # The Austrian swap-pair checks (engines/rp2.py:1722-1728).
    "validate": ("RP2 cross-asset input validation failed: ", "rp2_input_validation"),
    # The multi-asset hook, which also builds the accounting method
    # (engines/rp2.py:1344-1352, 622-630).
    "compute_multi": ("RP2 multi-asset tax calculation failed: ", "app_error"),
    # One asset's compute_tax (engines/rp2.py:1297-1300); the message names
    # the asset.
    "compute": ("RP2 tax calculation failed for asset '{asset}': ", "app_error"),
    # An Austrian classification at the first failing fragment
    # (engines/native.py:442-443).
    "classify": None,
    # The engine could not read the request, or does not implement it.
    "request": None,
    "unsupported": None,
}


def _inner_error(inner: Mapping[str, Any]) -> Exception:
    if inner.get("class") == "AppError":
        # The method gate: the adapter's own error, with the default code.
        return AppError(str(inner.get("message")))
    return engine_error(inner)


def ledger_error(error: Mapping[str, Any]) -> Exception:
    """The exception for a ledger failure ``{"stage", "asset"?, "inner"}``.

    A wrapped stage returns the adapter's ``AppError`` with the stage's error
    as its cause, as ``raise ... from`` would leave it.
    """
    stage = str(error.get("stage"))
    inner = error.get("inner") or {}
    if stage not in STAGES:
        return EngineContractError("RequestError", f"unknown ledger failure stage {stage!r}")
    if stage in ("request", "unsupported"):
        return EngineContractError(str(inner.get("class")), str(inner.get("message")))
    cause = _inner_error(inner)
    wrapper = STAGES[stage]
    if wrapper is None:
        return cause
    prefix, code = wrapper
    wrapped = AppError(prefix.format(asset=error.get("asset")) + str(cause), code=code)
    wrapped.__cause__ = cause
    wrapped.__suppress_context__ = True
    return wrapped


__all__ = [
    "ERROR_CLASSES",
    "STAGES",
    "EngineContractError",
    "EngineError",
    "EngineRuntimeError",
    "EngineTypeError",
    "EngineValueError",
    "engine_error",
    "ledger_error",
]
