"""Semantic comparison of tax-engine results across backends (plan 20).

The adapter (``kassiber/core/engines/rp2.py``) runs unchanged on either
calculation backend, so the comparison is exact: every value, its type, its
decimal representation (sign, coefficient and exponent), every key and every
order must match. Only two fields may differ, because they are generated or
name the backend: journal entry ids (``uuid4``) and the ``engine`` identity
in each entry's calculation metadata.

Failures compare too. Both backends must fail at the same point with the
same message, the same ``AppError`` fields, and the same wrapped error.
RP2's error classes count as their native counterparts, a listed difference
of plan 20.

``shadow_build_ledger_state`` backs ``KASSIBER_TEST_TAX_ENGINE=shadow`` in
``tests/conftest.py``.
"""

from __future__ import annotations

import dataclasses
import threading
from contextlib import contextmanager
from decimal import Decimal
from typing import Any, Callable, Iterator, Optional

from kassiber.core.engines import rp2 as adapter
from kassiber.core.engines.base import TaxEngineLedgerResult
from kassiber.errors import AppError

GENERATED = "<generated>"
MAX_DIFFERENCES = 40

# RP2's classes and the native backend's, which keep RP2's messages.
_NATIVE_ERROR_NAMES = {
    "RP2ValueError": "EngineValueError",
    "RP2TypeError": "EngineTypeError",
    "RP2RuntimeError": "EngineRuntimeError",
}

Outcome = tuple[Optional[Any], Optional[BaseException]]


def value_differences(expected: Any, actual: Any, path: str = "result") -> list[str]:
    """Every place where ``actual`` differs from ``expected``, by path."""
    differences: list[str] = []
    _diff(path, expected, actual, differences)
    return differences


def _diff(path: str, expected: Any, actual: Any, out: list[str]) -> None:
    if len(out) >= MAX_DIFFERENCES:
        return
    if type(expected) is not type(actual):
        out.append(
            f"{path}: {type(expected).__name__} {expected!r} != {type(actual).__name__} {actual!r}"
        )
        return
    if isinstance(expected, Decimal):
        # Equal values can still render differently: 3.0E+2 is not 300.
        if expected.as_tuple() != actual.as_tuple():
            out.append(f"{path}: {expected!r} != {actual!r}")
        return
    if isinstance(expected, float):
        # repr distinguishes -0.0 from 0.0 and matches NaN with NaN.
        if repr(expected) != repr(actual):
            out.append(f"{path}: {expected!r} != {actual!r}")
        return
    if isinstance(expected, dict):
        if list(expected) != list(actual):
            out.append(f"{path}: keys {list(expected)!r} != {list(actual)!r}")
        for key, value in expected.items():
            if key in actual:
                _diff(f"{path}[{key!r}]", value, actual[key], out)
        return
    if isinstance(expected, (list, tuple)):
        if len(expected) != len(actual):
            out.append(f"{path}: length {len(expected)} != {len(actual)}")
        for index, (left, right) in enumerate(zip(expected, actual)):
            _diff(f"{path}[{index}]", left, right, out)
        return
    if dataclasses.is_dataclass(expected) and not isinstance(expected, type):
        for field in dataclasses.fields(expected):
            _diff(f"{path}.{field.name}", getattr(expected, field.name), getattr(actual, field.name), out)
        return
    if expected != actual:
        out.append(f"{path}: {expected!r} != {actual!r}")


def _without_generated(entry: dict[str, Any]) -> dict[str, Any]:
    entry = dict(entry)
    if "id" in entry:
        entry["id"] = GENERATED
    calculation = entry.get("calculation")
    if isinstance(calculation, dict) and "engine" in calculation:
        entry["calculation"] = {**calculation, "engine": GENERATED}
    return entry


def comparable_ledger(result: TaxEngineLedgerResult) -> dict[str, Any]:
    """``result`` as a document, with the generated fields masked."""
    document = {field.name: getattr(result, field.name) for field in dataclasses.fields(result)}
    document["entries"] = [_without_generated(entry) for entry in result.entries]
    return document


def error_signature(error: Optional[BaseException]) -> Optional[tuple[Any, ...]]:
    """What must match between two failures.

    An ``AppError`` carries its fields and the error it wraps. An engine
    error carries its class and message; what RP2 chained inside it (such as
    dateutil's parser error) is not part of the output.
    """
    if error is None:
        return None
    name = type(error).__name__
    signature: list[Any] = [_NATIVE_ERROR_NAMES.get(name, name), str(error)]
    if isinstance(error, AppError):
        signature += [error.code, error.hint, error.details, error.retryable, error_signature(error.__cause__)]
    return tuple(signature)


def run_backend(name: str, call: Callable[[], Any]) -> Outcome:
    """Run ``call`` on backend ``name``; return its result or its error."""
    with adapter._use_tax_engine_backend(name):
        try:
            return call(), None
        except Exception as exc:  # noqa: BLE001 - failures are compared too
            return None, exc


def outcome_differences(
    expected: Outcome,
    actual: Outcome,
    compare: Callable[[Any, Any], list[str]],
) -> list[str]:
    """Differences between two outcomes; ``compare`` handles two results."""
    expected_result, expected_error = expected
    actual_result, actual_error = actual
    if expected_error is None and actual_error is None:
        return compare(expected_result, actual_result)
    expected_signature = error_signature(expected_error)
    actual_signature = error_signature(actual_error)
    if expected_signature == actual_signature:
        return []
    return [f"error: rp2 {expected_signature!r} != native {actual_signature!r}"]


def ledger_differences(expected: Outcome, actual: Outcome) -> list[str]:
    """Differences between two ``build_ledger_state`` outcomes."""
    return outcome_differences(
        expected,
        actual,
        lambda left, right: value_differences(comparable_ledger(left), comparable_ledger(right)),
    )


def run_both(engine: adapter.GenericRP2TaxEngine, inputs: Any) -> tuple[Outcome, Outcome]:
    """``engine.build_ledger_state(inputs)`` on RP2, then on the native backend."""
    build = _ORIGINAL_BUILD_LEDGER_STATE
    return (
        run_backend("rp2", lambda: build(engine, inputs)),
        run_backend("native", lambda: build(engine, inputs)),
    )


_ORIGINAL_BUILD_LEDGER_STATE = adapter.GenericRP2TaxEngine.build_ledger_state
# The backend switch is process-wide; one shadow call at a time keeps a
# concurrent caller from running on a half-switched backend.
_SHADOW_LOCK = threading.RLock()


@contextmanager
def shadow_build_ledger_state() -> Iterator[None]:
    """Make every ``build_ledger_state`` call run both backends.

    The call fails with ``AssertionError`` on any semantic difference;
    otherwise it returns (or raises) RP2's outcome, so the calling test
    still asserts against the product default.
    """
    engine_class = adapter.GenericRP2TaxEngine
    replaced = engine_class.build_ledger_state

    def build_ledger_state(self: adapter.GenericRP2TaxEngine, inputs: Any) -> TaxEngineLedgerResult:
        with _SHADOW_LOCK:
            expected, actual = run_both(self, inputs)
        differences = ledger_differences(expected, actual)
        if differences:
            raise AssertionError(
                "native tax engine differs from RP2 in build_ledger_state:\n  " + "\n  ".join(differences)
            )
        result, error = expected
        if error is not None:
            raise error
        return result

    engine_class.build_ledger_state = build_ledger_state
    try:
        yield
    finally:
        engine_class.build_ledger_state = replaced


__all__ = [
    "GENERATED",
    "comparable_ledger",
    "error_signature",
    "ledger_differences",
    "outcome_differences",
    "run_backend",
    "run_both",
    "shadow_build_ledger_state",
    "value_differences",
]
