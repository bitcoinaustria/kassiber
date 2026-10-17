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

A third applies to production alone (LD-5): its request builder may refuse a
book with an ``EngineContractError`` naming a shape a finalized projection
cannot have, where ``tests/tax_oracle/projection_range.py`` (written from the
design, not from the builder) finds the same shape in the inputs. The oracle's
outcome is then not compared, and the session counts the event; it must stay
zero for the books the suite builds. There is no other rule; an unknown
difference always fails.

The projection-level goldens (``tests/test_tax_ledger_goldens.py``) store
outcomes as ``outcome_document``s: the oracle's with its failures before the
two listed differences (``oracle_error_signature``), production's required
one after them.

``install_tax_mode`` installs the ``KASSIBER_TEST_TAX_ENGINE`` modes for
``tests/conftest.py`` and, in Python child processes such as the CLI and the
daemon, for ``tests/_child_hooks/sitecustomize.py``. ``shadow`` and
``census`` append one route record per production call to the session's
records file, which the session reads back at its end (``session_report``).
A record names the capability the book exercises (``classify``) and whether
the call reached the engine.
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
from typing import Any, Callable, ContextManager, Iterable, Iterator, Mapping, Optional, Sequence

from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines import KassiberTaxEngine
from kassiber.core.engines.base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from kassiber.core.engines.errors import EngineContractError
from kassiber.core.engines.ledger_request import build_request
from kassiber.errors import AppError
from kassiber.tax_policy import normalize_tax_country, profile_value

GENERATED = "<generated>"
MAX_DIFFERENCES = 40
# The engine class production journals run on.
PRODUCTION_ENGINE = KassiberTaxEngine
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

# The engine capabilities a book exercises, which the shadow summary counts
# calls by:
# - ``generic``: a generic book;
# - ``generic_carry``: a generic book with a carrying-value Bitcoin rail pair
#   whose legs are projection rows (the carry loop);
# - ``at``: an Austrian book;
# - ``at_swap``: an Austrian book with a carrying-value cross-asset pair whose
#   legs are projection rows (swap-link selection).
CAPABILITIES = ("generic", "generic_carry", "at", "at_swap")
# A call raw rows or a refused book end before the request shows its
# capabilities.
UNCLASSIFIED = "unclassified"
# The routes: the engine's schema-2 ``ledger`` operation, or none (a
# projection without rows, raw rows, or a refused book end before it).
V2 = "v2"
NOT_ROUTED = "none"
# The outcome of a call production's builder refused under LD-5.
LD5 = "ld5"

# LD-4: RP2's classes and the native backend's, which keep RP2's messages.
_NATIVE_ERROR_NAMES = {
    "RP2ValueError": "EngineValueError",
    "RP2TypeError": "EngineTypeError",
    "RP2RuntimeError": "EngineRuntimeError",
}
# LD-3: RP2 reads its configuration from a temporary INI file and prefixes
# the file's path to its configuration messages.
_INI_PREFIX = re.compile(r"^.*?\.ini: ")
# The goldens store the oracle's failures before LD-3 and LD-4, but the INI
# file's path is random, so it is stored as this placeholder.
STORED_INI_PATH = "<RP2 configuration file>.ini"

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


def oracle_error_signature(error: Optional[BaseException]) -> Optional[tuple[Any, ...]]:
    """``error_signature`` without LD-3 and LD-4: what the goldens store for
    an oracle failure.

    The class names stay RP2's and a configuration message keeps its INI
    prefix, with the random path replaced by ``STORED_INI_PATH``.
    ``listed_signature`` of the result is ``error_signature(error)``.
    """
    if error is None:
        return None
    message = _INI_PREFIX.sub(lambda _match: f"{STORED_INI_PATH}: ", str(error), count=1)
    signature: list[Any] = [type(error).__name__, message]
    if isinstance(error, AppError):
        signature += [error.code, error.hint, error.details, error.retryable, oracle_error_signature(error.__cause__)]
    return tuple(signature)


def listed_signature(signature: Optional[Sequence[Any]]) -> Optional[tuple[Any, ...]]:
    """Apply LD-4 and LD-3 to a stored ``oracle_error_signature``."""
    if signature is None:
        return None
    name, message, *fields = signature
    listed: list[Any] = [_NATIVE_ERROR_NAMES.get(name, name), _INI_PREFIX.sub("", message, count=1)]
    if fields:
        *app_fields, cause = fields
        listed += [*app_fields, listed_signature(cause)]
    return tuple(listed)


def outcome_document(
    outcome: Outcome,
    signature: Callable[[Optional[BaseException]], Optional[tuple[Any, ...]]] = error_signature,
) -> dict[str, Any]:
    """An outcome as the goldens store it: the masked ledger or the failure's signature."""
    result, error = outcome
    if error is None:
        return {"result": comparable_ledger(result)}
    return {"error": signature(error)}


def document_differences(expected: Mapping[str, Any], actual: Mapping[str, Any]) -> list[str]:
    """Differences between two ``outcome_document``s, by the same rule as ``ledger_differences``."""
    if "result" in expected and "result" in actual:
        return value_differences(expected["result"], actual["result"])
    if expected.get("error") == actual.get("error"):
        return []
    return [
        f"error: expected {expected.get('error')!r} != actual {actual.get('error')!r}"
        + ("" if "error" in expected and "error" in actual else " (one side succeeded)")
    ]


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


def capabilities(request: Mapping[str, Any]) -> frozenset[str]:
    """The capabilities a book's request uses."""
    row_ids = {row["id"] for row in request["rows"]}
    carried = [
        pair
        for pair in request["cross_asset_pairs"]
        if pair["policy"] == "carrying-value" and pair["out_id"] in row_ids and pair["in_id"] in row_ids
    ]
    if request["policy"]["country"]["kind"] == "at":
        return frozenset({"at", "at_swap"} if carried else {"at"})
    if any(pair["bitcoin_rail"] for pair in carried):
        return frozenset({"generic", "generic_carry"})
    return frozenset({"generic"})


def primary_capability(book_capabilities: frozenset[str]) -> str:
    """The capability a call counts under: the most specific one the book uses."""
    for capability in ("at_swap", "at", "generic_carry", "generic"):
        if capability in book_capabilities:
            return capability
    return UNCLASSIFIED


def classify(profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> tuple[str, str]:
    """The capability a ``build_ledger_state`` call counts under, and its route.

    A call that does not reach the engine counts as ``none``: under its
    country's base capability for a projection without rows, and as
    ``unclassified`` for raw rows or a book the builder refuses.
    """
    projection = inputs.finalized_tax_projection
    if not isinstance(projection, FinalizedTaxProjection):
        return UNCLASSIFIED, NOT_ROUTED
    if not projection.rows:
        try:
            country = normalize_tax_country(profile_value(profile, "tax_country"))
        except ValueError:
            return UNCLASSIFIED, NOT_ROUTED
        return ("at" if country == "at" else "generic"), NOT_ROUTED
    try:
        request = build_request(profile, inputs)
    except Exception:  # noqa: BLE001 - the call raises the same before the engine
        return UNCLASSIFIED, NOT_ROUTED
    return primary_capability(capabilities(request)), V2


def production_route(engine: Any, inputs: TaxEngineLedgerInputs) -> tuple[str, str]:
    """The capability a production call counts under, and its route."""
    return classify(engine.profile, inputs)


def listed_refusal(profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs, outcome: Outcome) -> Optional[str]:
    """The LD-5 invariant production refused the book for, when the refusal is listed.

    It is listed when production raised the builder's ``EngineContractError``
    for an invariant that ``projection_range`` also finds in the inputs.
    """
    _result, error = outcome
    invariant = getattr(error, "invariant", None) if isinstance(error, EngineContractError) else None
    if invariant is None:
        return None
    from tests.tax_oracle import projection_range

    return invariant if invariant in projection_range.violations(profile, inputs) else None


@dataclasses.dataclass(frozen=True)
class RouteRecord:
    module: Optional[str]
    capability: str
    route: str  # "v2" or "none"
    outcome: str  # "result", "error", "difference", "uncompared", or "ld5"


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
) -> tuple[list[str], list[str]]:
    """The summary lines and failures of a session's route records.

    Shadow counts the calls of each capability that reached the engine, and
    the calls that ended before it. It fails on any call that differed from
    the oracle, on a capability that saw calls but none on the engine, on a
    record of any other route, and on any LD-5 refusal (the books the suite
    builds are finalized projections). Census fails when a test module
    reached the engine but is not in ``overlay`` (test paths relative to the
    repository), or when a call ran outside any test.
    """
    records = list(records)
    lines: list[str] = []
    failures: list[str] = []
    if mode == SHADOW:
        compared = sum(1 for record in records if record.outcome not in ("uncompared", LD5))
        lines.append(f"production calls compared with the oracle: {compared}")
        routes = Counter((record.capability, record.route) for record in records)
        extra = sorted({record.capability for record in records} - set(CAPABILITIES))
        for capability in (*CAPABILITIES, *extra):
            v2, unrouted = (routes[(capability, route)] for route in (V2, NOT_ROUTED))
            lines.append(f"{capability}: {v2} v2" + (f", {unrouted} not routed" if unrouted else ""))
            if capability in CAPABILITIES and unrouted and not v2:
                failures.append(f"capability {capability!r} saw {unrouted} calls but none on v2")
        unknown = [record for record in records if record.route not in (V2, NOT_ROUTED)]
        if unknown:
            routes_seen = sorted({record.route for record in unknown})
            failures.append(f"calls on a route other than v2 or none: {len(unknown)}, on {', '.join(routes_seen)}")
        refused = [record for record in records if record.outcome == LD5]
        lines.append(f"LD-5 refusals (shapes a finalized projection cannot have): {len(refused)}")
        if refused:
            modules = sorted({record.module or "outside any test" for record in refused})
            failures.append(f"calls production's builder refused under LD-5: {len(refused)}, in {', '.join(modules)}")
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
            if listed_refusal(self.profile, inputs, actual) is not None:
                # LD-5: the oracle's outcome is not compared; the session
                # counts the refusal.
                record_route(capability, route, LD5)
                raise actual[1]  # type: ignore[misc]
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
    "LD5",
    "LEGACY_NATIVE",
    "LEGACY_NATIVE_BACKEND",
    "MODULE_ENV",
    "NOT_ROUTED",
    "ORACLE",
    "ORACLE_BACKEND",
    "PRODUCTION_ENGINE",
    "RECORDING_MODES",
    "RECORDS_ENV",
    "SHADOW",
    "STORED_INI_PATH",
    "TAX_MODES",
    "UNCLASSIFIED",
    "V2",
    "RouteRecord",
    "capabilities",
    "census_build_ledger_state",
    "classify",
    "comparable_ledger",
    "document_differences",
    "error_signature",
    "install_tax_mode",
    "ledger_differences",
    "listed_refusal",
    "listed_signature",
    "localize",
    "oracle_build_ledger_state",
    "oracle_error_signature",
    "outcome_differences",
    "outcome_document",
    "primary_capability",
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
