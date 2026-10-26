"""Keep the test session out of the per-user owner-lock namespace.

Every temp book a test opens leaves a lock group in the account-home namespace
behind for good, and one gate run creates thousands of throwaway ones.
Ownership exclusion deliberately follows no environment override, so the
namespace cannot be redirected for the child processes the cross-process tests
spawn; the groups this session created are swept when it ends instead. A file
another process still holds is skipped, so a concurrent preview or CLI keeps
its exclusion.
"""

from __future__ import annotations

import contextlib
import os
import tempfile
import time
from pathlib import Path

import pytest

from kassiber.operator import project as project_module
from tests.integration.env import env_flag, no_egress_guard


_ROOT = Path(__file__).resolve().parent.parent
_EGRESS_STACK: contextlib.ExitStack | None = None
_TAX_ENGINE_MODE: str | None = None
# Set in shadow and census, the modes that record each production call.
_TAX_MODULE_ENV: str | None = None
_PREVIOUS_TEST_NO_EGRESS: str | None = None
_PREVIOUS_PYTHONPATH: str | None = None
_CHILD_HOOKS_ARMED = False
_PREVIOUS_OFFLINE_PREFERENCE: str | None = None
_OFFLINE_PREFERENCE_DIR: tempfile.TemporaryDirectory | None = None
_TAX_RECORDS_DIR: tempfile.TemporaryDirectory | None = None
_PREVIOUS_TAX_RECORDS: str | None = None
_TAX_REPORT: tuple[list[str], list[str]] | None = None


def _isolate_offline_preference() -> None:
    """Point offline mode at a throwaway file for the whole session.

    The real preference lives in the user's state root, so a developer who
    left the app offline would otherwise see transport tests fail, and daemon
    subprocesses inherit the variable too.
    """
    global _PREVIOUS_OFFLINE_PREFERENCE, _OFFLINE_PREFERENCE_DIR
    if _OFFLINE_PREFERENCE_DIR is not None:
        return
    _PREVIOUS_OFFLINE_PREFERENCE = os.environ.get("KASSIBER_OFFLINE_PREFERENCE_FILE")
    _OFFLINE_PREFERENCE_DIR = tempfile.TemporaryDirectory(prefix="kassiber-offline-")
    os.environ["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(
        Path(_OFFLINE_PREFERENCE_DIR.name) / "offline-mode.json"
    )


def _tax_engine_mode() -> str:
    return os.environ.get("KASSIBER_TEST_TAX_ENGINE", "").strip().lower()


def _select_tax_engine_backend(config: pytest.Config) -> None:
    """Route the session's tax calculations per `KASSIBER_TEST_TAX_ENGINE`.

    Unset runs production. `shadow` also runs the frozen oracle
    (`tests/tax_oracle/`) on every production `build_ledger_state` call and
    fails on any difference (`tests/tax_engine_compare.py`). `census` runs
    production and fails the session when a test module that reached the
    engine is missing from the shadow overlay (`scripts/python_test_shards.py`).
    `oracle` runs every call on the oracle instead, to show the tests'
    expectations hold on RP2. Product code reads no such variable;
    `tests/_child_hooks/` carries `shadow` and `census` into Python child
    processes, while `oracle` leaves children on production.

    `shadow` and `census` record each call in a file the session creates
    here; xdist workers and child processes inherit its path.
    """
    global _TAX_ENGINE_MODE, _TAX_MODULE_ENV, _TAX_RECORDS_DIR, _PREVIOUS_TAX_RECORDS
    mode = _tax_engine_mode()
    if not mode or _TAX_ENGINE_MODE is not None:
        return
    from tests import tax_engine_compare as compare

    if mode not in compare.TAX_MODES:
        raise pytest.UsageError(
            f"KASSIBER_TEST_TAX_ENGINE must be shadow, census, or oracle, not {mode!r}"
        )
    if mode in compare.RECORDING_MODES:
        _TAX_MODULE_ENV = compare.MODULE_ENV
        if not hasattr(config, "workerinput"):
            _TAX_RECORDS_DIR = tempfile.TemporaryDirectory(prefix="kassiber-tax-shadow-")
            records = Path(_TAX_RECORDS_DIR.name) / "routes.jsonl"
            records.touch()
            _PREVIOUS_TAX_RECORDS = os.environ.get(compare.RECORDS_ENV)
            os.environ[compare.RECORDS_ENV] = str(records)
    compare.install_tax_mode(mode)
    _TAX_ENGINE_MODE = mode


def _arm_child_hooks() -> None:
    """Put `tests/_child_hooks/` and the repository on the children's path."""
    global _PREVIOUS_PYTHONPATH, _CHILD_HOOKS_ARMED
    if _CHILD_HOOKS_ARMED:
        return
    _PREVIOUS_PYTHONPATH = os.environ.get("PYTHONPATH")
    os.environ["PYTHONPATH"] = os.pathsep.join(
        [
            str(_ROOT / "tests" / "_child_hooks"),
            str(_ROOT),
            *([_PREVIOUS_PYTHONPATH] if _PREVIOUS_PYTHONPATH else []),
        ]
    )
    _CHILD_HOOKS_ARMED = True


def pytest_configure(config: pytest.Config) -> None:
    """Fail any test that tries to leave the machine.

    The suite's other no-network tests assert `not_called()` on one patched
    function each, so they only notice the path they were written for. This
    catches the ones nobody wrote a test for -- including egress that skips
    the ledger entirely, which is how the release check went unrecorded.

    `PYTHONPATH` carries it into the daemon subprocess the smoke tests spawn:
    `Popen` runs without `env=`, so the child inherits the variable and `site`
    imports `tests/_child_hooks/sitecustomize.py` before the daemon starts.
    A tax engine mode arms the same hook for its own reasons, also under
    `KASSIBER_INTEGRATION`.

    It sets `KASSIBER_TEST_NO_EGRESS`, not `KASSIBER_NO_EGRESS`. The latter is
    a product kill switch that the BDK and LWK observers honor
    destination-blind -- setting it here would make them refuse the loopback
    fakes the smoke tests serve, which is a different thing than "do not leave
    the machine".

    Loopback stays allowed: ~36 daemon smoke tests bind local
    `ThreadingHTTPServer` fakes. So the invariant's "including loopback
    service/provider probes" clause is *not* covered here -- a test that
    needs that asserts it directly.
    """
    config.addinivalue_line(
        "markers",
        "stubs_tax_engine: the test replaces parts of the production tax engine, so the shadow mode runs "
        "its calls on production without comparing them with the oracle",
    )
    _isolate_offline_preference()
    _select_tax_engine_backend(config)
    integration = env_flag("KASSIBER_INTEGRATION")
    if _TAX_ENGINE_MODE is not None or not integration:
        _arm_child_hooks()
    if integration:
        return

    global _EGRESS_STACK, _PREVIOUS_TEST_NO_EGRESS
    if _EGRESS_STACK is not None:
        return

    _PREVIOUS_TEST_NO_EGRESS = os.environ.get("KASSIBER_TEST_NO_EGRESS")
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    _EGRESS_STACK = contextlib.ExitStack()
    _EGRESS_STACK.enter_context(no_egress_guard(enabled=True))


def _test_module(item: pytest.Item) -> str:
    path = Path(item.path).resolve()
    try:
        return path.relative_to(_ROOT).as_posix()
    except ValueError:
        return path.as_posix()


@pytest.hookimpl(wrapper=True)
def pytest_runtest_protocol(item: pytest.Item, nextitem: pytest.Item | None):
    """Name the running test module for the route records of shadow and census.

    An environment variable rather than a global, so the child processes the
    test spawns attribute their calls to it too. A `stubs_tax_engine` test
    runs uncompared under shadow.
    """
    if _TAX_MODULE_ENV is None:
        return (yield)
    from tests import tax_engine_compare as compare

    previous = os.environ.get(_TAX_MODULE_ENV)
    os.environ[_TAX_MODULE_ENV] = _test_module(item)
    stubbed = item.get_closest_marker("stubs_tax_engine") is not None
    try:
        with compare.uncompared() if stubbed else contextlib.nullcontext():
            return (yield)
    finally:
        if previous is None:
            os.environ.pop(_TAX_MODULE_ENV, None)
        else:
            os.environ[_TAX_MODULE_ENV] = previous


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    """Check the session's route records, in the process that created them."""
    global _TAX_REPORT
    if _TAX_RECORDS_DIR is None:
        return
    from scripts.python_test_shards import shadow_overlay
    from tests import tax_engine_compare as compare

    records = compare.read_route_records(os.environ[compare.RECORDS_ENV])
    overlay = [path.as_posix() for path in shadow_overlay(_ROOT)] if _TAX_ENGINE_MODE == compare.CENSUS else []
    _TAX_REPORT = compare.session_report(_TAX_ENGINE_MODE, records, overlay=overlay)
    if _TAX_REPORT[1] and session.exitstatus in (pytest.ExitCode.OK, pytest.ExitCode.NO_TESTS_COLLECTED):
        session.exitstatus = pytest.ExitCode.TESTS_FAILED


def pytest_terminal_summary(terminalreporter, exitstatus: int, config: pytest.Config) -> None:
    if _TAX_REPORT is None:
        return
    lines, failures = _TAX_REPORT
    terminalreporter.write_sep("-", f"tax engine {_TAX_ENGINE_MODE}")
    for line in lines:
        terminalreporter.write_line(line)
    for failure in failures:
        terminalreporter.write_line(f"FAILED: {failure}", red=True, bold=True)


def pytest_unconfigure(config: pytest.Config) -> None:
    global _EGRESS_STACK, _OFFLINE_PREFERENCE_DIR, _TAX_ENGINE_MODE, _TAX_MODULE_ENV, _TAX_RECORDS_DIR
    global _CHILD_HOOKS_ARMED
    if _TAX_ENGINE_MODE is not None:
        from tests import tax_engine_compare as compare

        compare.uninstall_tax_mode()
        _TAX_ENGINE_MODE = None
        _TAX_MODULE_ENV = None
        if _TAX_RECORDS_DIR is not None:
            if _PREVIOUS_TAX_RECORDS is None:
                os.environ.pop(compare.RECORDS_ENV, None)
            else:
                os.environ[compare.RECORDS_ENV] = _PREVIOUS_TAX_RECORDS
            _TAX_RECORDS_DIR.cleanup()
            _TAX_RECORDS_DIR = None
    if _OFFLINE_PREFERENCE_DIR is not None:
        if _PREVIOUS_OFFLINE_PREFERENCE is None:
            os.environ.pop("KASSIBER_OFFLINE_PREFERENCE_FILE", None)
        else:
            os.environ["KASSIBER_OFFLINE_PREFERENCE_FILE"] = _PREVIOUS_OFFLINE_PREFERENCE
        _OFFLINE_PREFERENCE_DIR.cleanup()
        _OFFLINE_PREFERENCE_DIR = None
    if _CHILD_HOOKS_ARMED:
        _CHILD_HOOKS_ARMED = False
        if _PREVIOUS_PYTHONPATH is None:
            os.environ.pop("PYTHONPATH", None)
        else:
            os.environ["PYTHONPATH"] = _PREVIOUS_PYTHONPATH
    if _EGRESS_STACK is None:
        return
    _EGRESS_STACK.close()
    _EGRESS_STACK = None
    if _PREVIOUS_TEST_NO_EGRESS is None:
        os.environ.pop("KASSIBER_TEST_NO_EGRESS", None)
    else:
        os.environ["KASSIBER_TEST_NO_EGRESS"] = _PREVIOUS_TEST_NO_EGRESS


@pytest.fixture(autouse=True, scope="session")
def _sweep_session_owner_locks():
    # Filesystem timestamps are coarser than time.time(), so a file created
    # just after the session starts can carry a slightly earlier mtime.
    started = time.time() - 5.0
    yield
    if os.name == "nt":
        # Exclusion is decided by share modes at open time, so probing a live
        # lock would itself fail rather than report the holder.
        return
    try:
        root = project_module._owner_lock_root()
    except Exception:
        return
    for path in root.glob("*.lock*"):
        try:
            if path.stat().st_mtime < started:
                continue
            handle = project_module._open_owner_lock(path, "sweep")
        except Exception:
            continue
        try:
            # ponytail: unlink under our own lock only, not the group's
            # admission lock. A holder mid-acquire on a group this session
            # created could in principle end up on a replaced inode; take the
            # admission lock per group if that ever shows up.
            if project_module._try_lock_handle(handle):
                path.unlink(missing_ok=True)
        except OSError:
            # One file that cannot be removed must not abort the sweep, and a
            # session that already passed must not fail in teardown over it.
            continue
        finally:
            try:
                project_module._unlock_handle(handle)
            finally:
                handle.close()
