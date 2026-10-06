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

``install_tax_mode`` installs the ``KASSIBER_TEST_TAX_ENGINE`` modes for
``tests/conftest.py`` and, in Python child processes such as the CLI and the
daemon, for ``tests/_child_hooks/sitecustomize.py``. ``shadow`` and
``census`` append one route record per production call to the session's
records file, which the session reads back at its end (``session_report``).
"""

from __future__ import annotations

import dataclasses
import json
import os
import re
import threading
from collections import Counter
from contextlib import ExitStack, contextmanager
from decimal import Decimal
from typing import Any, Callable, ContextManager, Iterable, Iterator, Mapping, Optional

from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.core.engines.base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from kassiber.errors import AppError

GENERATED = "<generated>"
MAX_DIFFERENCES = 40
# The engine class production journals run on.
PRODUCTION_ENGINE = GenericRP2TaxEngine
# The oracle's calculation backends: RP2, and the legacy-native leg (the
# oracle adapter on ``kassiber/core/engines/native.py``) that localizes a
# difference to the fold or the lot engine.
ORACLE_BACKEND = "rp2"
LEGACY_NATIVE_BACKEND = "native"

# The values of KASSIBER_TEST_TAX_ENGINE.
SHADOW = "shadow"
CENSUS = "census"
ORACLE = "oracle"
LEGACY_NATIVE = "legacy-native"
TAX_MODES = (SHADOW, CENSUS, ORACLE, LEGACY_NATIVE)
# The modes that record every production call. Child processes install
# them too.
RECORDING_MODES = (SHADOW, CENSUS)

# Route records: one JSON line per production call, appended to the file
# this variable names by the session and its workers and child processes.
# A record holds the calling test module, the capability, the route, and
# whether the call returned, raised, or differed from the oracle; nothing
# from the book.
RECORDS_ENV = "KASSIBER_TEST_TAX_SHADOW_RECORDS"
# The test module running, set around each test so child processes inherit it.
MODULE_ENV = "KASSIBER_TEST_TAX_SHADOW_MODULE"
# Test-only: shadow alters production's outcome before comparing, which proves
# that a child process compares (tests/test_tax_shadow_harness.py).
INJECT_DIFFERENCE_ENV = "KASSIBER_TEST_TAX_SHADOW_INJECT_DIFFERENCE"

# The book capabilities production routes by.
CAPABILITIES = ("generic", "generic_carry", "at", "at_swap")
# Production does not classify books yet; every call runs the phase-3 adapter.
UNCLASSIFIED = "unclassified"
# The capabilities production sends to the v2 engine: none yet.
ROUTED_CAPABILITIES: frozenset[str] = frozenset()

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


def _oracle_adapter() -> Any:
    """The frozen oracle adapter, imported on first use.

    The child hook installs a mode as soon as ``kassiber.core.engines`` has
    loaded, which can happen while the oracle adapter is still importing,
    so this module must not import the oracle at its top.
    """
    from tests.tax_oracle import adapter

    return adapter


def run_on_oracle_backend(name: str, call: Callable[[], Any]) -> Outcome:
    """Run ``call`` with the oracle adapter on backend ``name``; return its result or its error."""
    with _oracle_adapter()._use_tax_engine_backend(name):
        return _outcome(call)


def run_oracle(
    profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    *,
    backend: str = ORACLE_BACKEND,
) -> Outcome:
    """The oracle's ``build_ledger_state`` outcome, on RP2 unless ``backend`` says otherwise."""
    return run_on_oracle_backend(backend, lambda: _oracle_adapter().OracleTaxEngine(profile).build_ledger_state(inputs))


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


def production_route(engine: Any, inputs: TaxEngineLedgerInputs) -> tuple[str, str]:
    """The capability a production call counts under, and its route.

    Production has no capability routing yet, so every call is unclassified
    and runs the legacy (phase-3) adapter.
    """
    return UNCLASSIFIED, "legacy"


@dataclasses.dataclass(frozen=True)
class RouteRecord:
    module: Optional[str]
    capability: str
    route: str  # "v2" or "legacy"
    outcome: str  # "result", "error", "difference", or "uncompared"


def record_route(capability: str, route: str, outcome: str) -> None:
    """Append one route record, when the session keeps a records file."""
    path = os.environ.get(RECORDS_ENV)
    if not path:
        return
    line = json.dumps(
        {
            "capability": capability,
            "module": os.environ.get(MODULE_ENV) or None,
            "outcome": outcome,
            "route": route,
        },
        sort_keys=True,
    )
    # One short write per record, so concurrent workers and child processes
    # never interleave inside a line.
    with open(path, "a", encoding="utf-8") as handle:
        handle.write(line + "\n")


def read_route_records(path: str) -> list[RouteRecord]:
    records = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if line.strip():
                record = json.loads(line)
                records.append(
                    RouteRecord(record["module"], record["capability"], record["route"], record["outcome"])
                )
    return records


def session_report(
    mode: str,
    records: Iterable[RouteRecord],
    *,
    overlay: Iterable[str] = (),
    routed: frozenset[str] = ROUTED_CAPABILITIES,
) -> tuple[list[str], list[str]]:
    """The summary lines and failures of a session's route records.

    Shadow counts the v2 and legacy calls of each capability. It fails on
    any call that differed from the oracle, and on any call of a routed
    capability that went legacy (which includes a routed capability that
    never reached v2). Census fails when a test module reached the engine
    but is not in ``overlay`` (test paths relative to the repository), or
    when a call ran outside any test.
    """
    records = list(records)
    lines: list[str] = []
    failures: list[str] = []
    if mode == SHADOW:
        compared = sum(1 for record in records if record.outcome != "uncompared")
        lines.append(f"production calls compared with the oracle: {compared}")
        routes = Counter((record.capability, record.route) for record in records)
        extra = sorted({record.capability for record in records} - set(CAPABILITIES))
        for capability in (*CAPABILITIES, *extra):
            v2, legacy = routes[(capability, "v2")], routes[(capability, "legacy")]
            lines.append(f"{capability}: {v2} v2, {legacy} legacy")
            if capability in routed and legacy:
                failures.append(
                    f"{legacy} of {v2 + legacy} calls of routed capability {capability!r} went to the legacy adapter"
                )
        uncompared = [record for record in records if record.outcome == "uncompared"]
        if uncompared:
            modules = sorted({record.module or "outside any test" for record in uncompared})
            lines.append(
                f"calls not compared because the test stubs the engine: {len(uncompared)}, in {', '.join(modules)}"
            )
        differing = [record for record in records if record.outcome == "difference"]
        if differing:
            modules = sorted({record.module or "outside any test" for record in differing})
            failures.append(f"calls that differed from the oracle: {len(differing)}, in {', '.join(modules)}")
    elif mode == CENSUS:
        modules = {record.module for record in records if record.module}
        lines.append(f"production calls: {len(records)}, from test modules: {len(modules)}")
        missing = sorted(modules - set(overlay))
        if missing:
            failures.append(
                "these test modules reach the tax engine but are not in the shadow overlay"
                " (scripts/python_test_shards.py): " + ", ".join(missing)
            )
        outside = sum(1 for record in records if not record.module)
        if outside:
            failures.append(f"production calls outside any test, which the census cannot place: {outside}")
    return lines, failures


def _with_injected_difference(outcome: Outcome) -> Outcome:
    result, error = outcome
    if error is not None:
        return None, RuntimeError(f"difference injected by {INJECT_DIFFERENCE_ENV}")
    return dataclasses.replace(result, tax_summary=[*result.tax_summary, {"injected_difference": True}]), None


# RP2 has process-global import side effects and the oracle's backend switch
# is process-wide; one shadow call at a time keeps a concurrent caller from
# running on a half-switched backend.
_SHADOW_LOCK = threading.RLock()
_UNCOMPARED = False


@contextmanager
def uncompared() -> Iterator[None]:
    """Run shadow's calls on production alone, recorded as ``uncompared``.

    For a test that replaces parts of the production engine (the
    ``stubs_tax_engine`` marker): what it calls is not production, so there
    is nothing to compare with the oracle.
    """
    global _UNCOMPARED
    previous, _UNCOMPARED = _UNCOMPARED, True
    try:
        yield
    finally:
        _UNCOMPARED = previous


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

    The call fails with ``AssertionError`` on any difference and records it,
    so the session fails even where the caller catches the error; otherwise
    it returns (or raises) production's outcome, so the calling test asserts
    against production.
    """

    def build_ledger_state(self: Any, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        capability, route = production_route(self, inputs)
        if _UNCOMPARED:
            result, error = _outcome(lambda: _PRODUCTION_BUILD_LEDGER_STATE(self, inputs))
            record_route(capability, route, "uncompared")
            if error is not None:
                raise error
            return result
        with _SHADOW_LOCK:
            expected = run_oracle(self.profile, inputs)
            actual = _outcome(lambda: _PRODUCTION_BUILD_LEDGER_STATE(self, inputs))
            injected = os.environ.get(INJECT_DIFFERENCE_ENV) == "1"
            if injected:
                actual = _with_injected_difference(actual)
            differences = ledger_differences(expected, actual)
            if differences:
                record_route(capability, route, "difference")
                raise AssertionError(
                    "production tax engine differs from the oracle in build_ledger_state ("
                    + localize(self.profile, inputs, expected, actual)
                    + (f"; injected by {INJECT_DIFFERENCE_ENV}" if injected else "")
                    + "):\n  "
                    + "\n  ".join(differences)
                )
        result, error = actual
        record_route(capability, route, "result" if error is None else "error")
        if error is not None:
            raise error
        return result

    with _patched_build_ledger_state(build_ledger_state):
        yield


@contextmanager
def census_build_ledger_state() -> Iterator[None]:
    """Record every production ``build_ledger_state`` call, and run production."""

    def build_ledger_state(self: Any, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        capability, route = production_route(self, inputs)
        result, error = _outcome(lambda: _PRODUCTION_BUILD_LEDGER_STATE(self, inputs))
        record_route(capability, route, "result" if error is None else "error")
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
        return _oracle_adapter().OracleTaxEngine(self.profile).build_ledger_state(inputs)

    with _oracle_adapter()._use_tax_engine_backend(backend), _patched_build_ledger_state(build_ledger_state):
        yield


def _tax_mode_patch(mode: str) -> ContextManager[None]:
    if mode == SHADOW:
        return shadow_build_ledger_state()
    if mode == CENSUS:
        return census_build_ledger_state()
    if mode == ORACLE:
        return oracle_build_ledger_state(ORACLE_BACKEND)
    if mode == LEGACY_NATIVE:
        return oracle_build_ledger_state(LEGACY_NATIVE_BACKEND)
    raise ValueError(f"KASSIBER_TEST_TAX_ENGINE must be one of {', '.join(TAX_MODES)}, not {mode!r}")


_INSTALLED_MODE: Optional[tuple[str, ExitStack]] = None


def install_tax_mode(mode: str) -> None:
    """Patch production for ``mode`` in this process until ``uninstall_tax_mode``.

    Installing the installed mode again does nothing: an xdist worker gets it
    from ``tests/conftest.py`` and from the child hook it inherits.
    """
    global _INSTALLED_MODE
    if _INSTALLED_MODE is not None:
        if _INSTALLED_MODE[0] != mode:
            raise RuntimeError(f"tax engine test mode {_INSTALLED_MODE[0]!r} is installed, not {mode!r}")
        return
    stack = ExitStack()
    stack.enter_context(_tax_mode_patch(mode))
    _INSTALLED_MODE = (mode, stack)


def uninstall_tax_mode() -> None:
    global _INSTALLED_MODE
    if _INSTALLED_MODE is not None:
        _INSTALLED_MODE[1].close()
        _INSTALLED_MODE = None


__all__ = [
    "CAPABILITIES",
    "CENSUS",
    "GENERATED",
    "INJECT_DIFFERENCE_ENV",
    "LEGACY_NATIVE",
    "LEGACY_NATIVE_BACKEND",
    "MODULE_ENV",
    "ORACLE",
    "ORACLE_BACKEND",
    "PRODUCTION_ENGINE",
    "RECORDING_MODES",
    "RECORDS_ENV",
    "ROUTED_CAPABILITIES",
    "SHADOW",
    "TAX_MODES",
    "UNCLASSIFIED",
    "RouteRecord",
    "census_build_ledger_state",
    "comparable_ledger",
    "error_signature",
    "install_tax_mode",
    "ledger_differences",
    "localize",
    "oracle_build_ledger_state",
    "outcome_differences",
    "production_route",
    "read_route_records",
    "record_route",
    "run_on_oracle_backend",
    "run_oracle",
    "run_production",
    "session_report",
    "shadow_build_ledger_state",
    "uncompared",
    "uninstall_tax_mode",
    "value_differences",
]
