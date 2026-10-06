"""Exact comparison of production's tax ledger with the frozen oracle (plan 20).

The oracle is the phase-3 adapter frozen in ``tests/tax_oracle/`` together
with its normalizer, Austrian tracker, pair allocation and policy, running on
RP2. Production is the engine class ``custody_journal.py`` instantiates.
Production must equal the oracle exactly: every value, its type, its decimal
representation (sign, coefficient and exponent), every key and every order.
Only two fields are masked, because they are generated or name the engine:
journal entry ids (``uuid4``) and the ``engine`` identity in each entry's
calculation metadata.

Failures compare too: the same class, message, ``AppError`` fields and
wrapped error. Two listed differences of plan 20 apply to both sides:

- RP2's error classes count as their ``Engine*`` counterparts (LD-4);
- RP2's configuration messages lose their temporary INI path prefix (LD-3).

There is no other rule; an unknown difference always fails.

``shadow_build_ledger_state`` backs ``KASSIBER_TEST_TAX_ENGINE=shadow`` and
``oracle_build_ledger_state`` backs ``oracle`` and ``legacy-native`` in
``tests/conftest.py``.
"""

from __future__ import annotations

import dataclasses
import re
import threading
from contextlib import contextmanager
from decimal import Decimal
from typing import Any, Callable, Iterator, Mapping, Optional

from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.core.engines.base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from kassiber.errors import AppError
from tests.tax_oracle import adapter as oracle_adapter
from tests.tax_oracle.adapter import OracleTaxEngine

GENERATED = "<generated>"
MAX_DIFFERENCES = 40
# The engine class production journals run on.
PRODUCTION_ENGINE = GenericRP2TaxEngine
# The oracle's calculation backends: RP2, and the legacy-native leg (the
# oracle adapter on ``kassiber/core/engines/native.py``) that localizes a
# difference to the fold or the lot engine.
ORACLE_BACKEND = "rp2"
LEGACY_NATIVE_BACKEND = "native"

# LD-4: RP2's classes and the native backend's, which keep RP2's messages.
_NATIVE_ERROR_NAMES = {
    "RP2ValueError": "EngineValueError",
    "RP2TypeError": "EngineTypeError",
    "RP2RuntimeError": "EngineRuntimeError",
}
# LD-3: RP2 reads its configuration from a temporary INI file and prefixes
# the file's path to its configuration messages.
_INI_PREFIX = re.compile(r"^.*?\.ini: ")

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
    message = _INI_PREFIX.sub("", str(error), count=1)
    signature: list[Any] = [_NATIVE_ERROR_NAMES.get(name, name), message]
    if isinstance(error, AppError):
        signature += [error.code, error.hint, error.details, error.retryable, error_signature(error.__cause__)]
    return tuple(signature)


def _outcome(call: Callable[[], Any]) -> Outcome:
    try:
        return call(), None
    except Exception as exc:  # noqa: BLE001 - failures are compared too
        return None, exc


def run_on_oracle_backend(name: str, call: Callable[[], Any]) -> Outcome:
    """Run ``call`` with the oracle adapter on backend ``name``; return its result or its error."""
    with oracle_adapter._use_tax_engine_backend(name):
        return _outcome(call)


def run_oracle(
    profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    *,
    backend: str = ORACLE_BACKEND,
) -> Outcome:
    """The oracle's ``build_ledger_state`` outcome, on RP2 unless ``backend`` says otherwise."""
    return run_on_oracle_backend(backend, lambda: OracleTaxEngine(profile).build_ledger_state(inputs))


_PRODUCTION_BUILD_LEDGER_STATE = PRODUCTION_ENGINE.build_ledger_state


def run_production(profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Outcome:
    """Production's ``build_ledger_state`` outcome, even while a test mode patches it."""
    return _outcome(lambda: _PRODUCTION_BUILD_LEDGER_STATE(PRODUCTION_ENGINE(profile), inputs))


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
    return [f"error: expected {expected_signature!r} != actual {actual_signature!r}"]


def ledger_differences(expected: Outcome, actual: Outcome) -> list[str]:
    """Differences between two ``build_ledger_state`` outcomes."""
    return outcome_differences(
        expected,
        actual,
        lambda left, right: value_differences(comparable_ledger(left), comparable_ledger(right)),
    )


def localize(
    profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    oracle: Outcome,
    production: Outcome,
) -> str:
    """Whether the fold or the lot engine explains a difference.

    The legacy-native leg runs the oracle's fold on the native lot engine.
    """
    legacy = run_oracle(profile, inputs, backend=LEGACY_NATIVE_BACKEND)
    if not ledger_differences(legacy, production):
        return "the oracle on the native lot engine equals production: the lot engines differ"
    if not ledger_differences(oracle, legacy):
        return "the oracle is the same on both lot engines: production's fold differs"
    return "the oracle differs between the lot engines, and production differs from both"


# RP2 has process-global import side effects and the oracle's backend switch
# is process-wide; one shadow call at a time keeps a concurrent caller from
# running on a half-switched backend.
_SHADOW_LOCK = threading.RLock()


@contextmanager
def _patched_build_ledger_state(build_ledger_state: Callable[[Any, TaxEngineLedgerInputs], Any]) -> Iterator[None]:
    replaced = PRODUCTION_ENGINE.build_ledger_state
    PRODUCTION_ENGINE.build_ledger_state = build_ledger_state
    try:
        yield
    finally:
        PRODUCTION_ENGINE.build_ledger_state = replaced


@contextmanager
def shadow_build_ledger_state() -> Iterator[None]:
    """Compare every production ``build_ledger_state`` call with the oracle.

    The call fails with ``AssertionError`` on any difference; otherwise it
    returns (or raises) production's outcome, so the calling test asserts
    against production.
    """

    def build_ledger_state(self: Any, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        with _SHADOW_LOCK:
            expected = run_oracle(self.profile, inputs)
            actual = _outcome(lambda: _PRODUCTION_BUILD_LEDGER_STATE(self, inputs))
            differences = ledger_differences(expected, actual)
            if differences:
                raise AssertionError(
                    "production tax engine differs from the oracle in build_ledger_state ("
                    + localize(self.profile, inputs, expected, actual)
                    + "):\n  "
                    + "\n  ".join(differences)
                )
        result, error = actual
        if error is not None:
            raise error
        return result

    with _patched_build_ledger_state(build_ledger_state):
        yield


@contextmanager
def oracle_build_ledger_state(backend: str = ORACLE_BACKEND) -> Iterator[None]:
    """Run every production ``build_ledger_state`` call on the oracle instead.

    ``backend`` stays selected for the oracle adapter while the patch is in
    place, so ``LEGACY_NATIVE_BACKEND`` runs the whole session on the
    legacy-native leg.
    """

    def build_ledger_state(self: Any, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        return OracleTaxEngine(self.profile).build_ledger_state(inputs)

    with oracle_adapter._use_tax_engine_backend(backend), _patched_build_ledger_state(build_ledger_state):
        yield


__all__ = [
    "GENERATED",
    "LEGACY_NATIVE_BACKEND",
    "ORACLE_BACKEND",
    "PRODUCTION_ENGINE",
    "comparable_ledger",
    "error_signature",
    "ledger_differences",
    "localize",
    "oracle_build_ledger_state",
    "outcome_differences",
    "run_on_oracle_backend",
    "run_oracle",
    "run_production",
    "shadow_build_ledger_state",
    "value_differences",
]
