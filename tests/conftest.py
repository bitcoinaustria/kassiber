"""Keep the test session out of the per-user owner-lock namespace.

Every temp book a test opens leaves a lock group in the account-home namespace
behind for good, and one gate run creates thousands of throwaway ones.
Ownership exclusion deliberately follows no environment override, so the
namespace cannot be redirected for the child processes the cross-process tests
spawn; the groups this session touched are swept when it ends instead, once,
in the process that started the run and after every xdist worker has ended.
The sweep follows the product's own pruner (tests/owner_lock_sweep.py): each
group's admission lock, so another run, the desktop or a CLI mid-acquire is
never moved onto an unlinked record; a file another process still holds is
skipped, so a concurrent preview or CLI keeps its exclusion.
"""

from __future__ import annotations

import contextlib
import os
import re
import tempfile
import time
import zlib
from pathlib import Path

import pytest

from kassiber.operator import project as project_module
from tests.integration.env import env_flag, no_egress_guard
from tests.owner_lock_sweep import sweep_owner_locks
from tests.rust_fixture_binary import stale_environment_binaries


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
_OWNER_LOCK_SWEEP_STARTED: float | None = None


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


def _record_custody_snapshots() -> None:
    """Write a snapshot of every custody build when asked to.

    With `KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR` set to an existing directory,
    each in-process `build_custody_projection` call writes its inputs and
    results there (`tests/custody_snapshot.py`). Product code reads no such
    variable.
    """
    if os.environ.get("KASSIBER_TEST_JOURNAL_RENDER_DIR"):
        from tests.rust_port import journal_render_oracle
        journal_render_oracle.install_from_environment()
    if not os.environ.get("KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR", "").strip():
        return
    from tests import custody_snapshot

    custody_snapshot.install_from_environment()


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


def _default_to_scope_distribution(config: pytest.Config) -> None:
    """Keep a test class on one worker under `-n`, unless `--dist` was given.

    Some classes are ordered steps over one shared book (the CLI smoke suite
    runs `test_00…` before `test_13…`). xdist's default `load` spreads a
    class over workers, so a plain `pytest -n 8` failed those steps with
    "Workspace 'Main' not found". CI shards already pass `--dist loadscope`;
    this makes a local parallel run do the same.
    """
    if not getattr(config.option, "numprocesses", None):
        return
    args = [*config.invocation_params.args, *os.environ.get("PYTEST_ADDOPTS", "").split()]
    if any(arg == "--dist" or arg.startswith("--dist=") or arg == "-d" for arg in args):
        return
    config.option.dist = "loadscope"


def _refuse_stale_binaries(config: pytest.Config) -> None:
    """Stop the run when a Rust binary the environment names is stale.

    An older binary's output reads as a port gap (tests/rust_fixture_binary.py);
    examples found beside `KASSIBER_RUST_BIN` are checked where they run.
    """
    if hasattr(config, "workerinput"):
        return
    stale = stale_environment_binaries()
    if stale:
        raise pytest.UsageError(
            "stale Rust test binaries; rebuild them with cargo:\n"
            + "\n".join(f"  {variable}={path} {reason}" for variable, path, reason in stale)
        )


def _require_test_data() -> None:
    """Refuse to run without the generated data tests/test-data.lock pins.

    scripts/fetch-test-data.sh puts the data in place and stamps the lock it
    fetched; a missing or older stamp would otherwise surface as scattered
    FileNotFoundErrors (docs/reference/testing.md#generated-test-data).
    """
    lock = _ROOT / "tests" / "test-data.lock"
    if not lock.exists():
        return
    import hashlib

    data = lock.read_bytes()
    blob = hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()
    stamp = _ROOT / ".test-data" / "stamp"
    stamped = stamp.read_text(encoding="utf-8").split() if stamp.exists() else []
    paths = [line.split(" ", 1)[1] for line in data.decode("utf-8").splitlines() if re.fullmatch(r"[0-9a-f]{40} \S+", line)]
    if blob not in stamped or not all((_ROOT / path).is_file() for path in paths):
        pytest.exit(
            "Generated test data is missing or older than tests/test-data.lock: run scripts/fetch-test-data.sh",
            returncode=4,
        )


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
    _require_test_data()
    config.addinivalue_line(
        "markers",
        "stubs_tax_engine: the test replaces parts of the production tax engine, so the shadow mode runs "
        "its calls on production without comparing them with the oracle",
    )
    _refuse_stale_binaries(config)
    _default_to_scope_distribution(config)
    _start_owner_lock_sweep(config)
    _isolate_offline_preference()
    _select_tax_engine_backend(config)
    _record_custody_snapshots()
    _select_custody_engine(config)
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


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--kassiber-part",
        default=None,
        metavar="K/N",
        help="Keep part K of N of the collected tests (scripts/python_test_shards.py --part).",
    )


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    """Keep one part of a lane split across CI jobs (`--kassiber-part K/N`).

    A test belongs to the part its node id's CRC-32 picks, so every xdist
    worker and every job agrees without coordination, and each test runs in
    exactly one part (scripts/python_test_shards.py `--part`). An option,
    not an environment variable, so a pytest a test starts keeps all of its
    own tests.
    """
    part = (config.getoption("kassiber_part") or "").strip()
    if not part:
        return
    index, slash, count = part.partition("/")
    if not (slash and index.isdigit() and count.isdigit() and 1 <= int(index) <= int(count)):
        raise pytest.UsageError(f"--kassiber-part must be K/N with 1 <= K <= N, not {part!r}")
    index, count = int(index), int(count)
    keep, drop = [], []
    for item in items:
        (keep if zlib.crc32(item.nodeid.encode("utf-8")) % count == index - 1 else drop).append(item)
    if drop:
        config.hook.pytest_deselected(items=drop)
        items[:] = keep


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
    if os.environ.get("KASSIBER_TEST_CUSTODY_ENGINE"):
        os.environ["KASSIBER_TEST_CUSTODY_MODULE"] = _test_module(item)
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
    _finish_custody_session(session)
    _finish_owner_lock_sweep()
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
    _custody_terminal_summary(terminalreporter)
    if _TAX_REPORT is None:
        return
    lines, failures = _TAX_REPORT
    terminalreporter.write_sep("-", f"tax engine {_TAX_ENGINE_MODE}")
    for line in lines:
        terminalreporter.write_line(line)
    for failure in failures:
        terminalreporter.write_line(f"FAILED: {failure}", red=True, bold=True)


def pytest_unconfigure(config: pytest.Config) -> None:
    _unconfigure_custody()
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


def _start_owner_lock_sweep(config: pytest.Config) -> None:
    """Arm the sweep in the process that started the run, never in a worker.

    A worker that drained early would otherwise sweep while its siblings still
    open books.
    """
    global _OWNER_LOCK_SWEEP_STARTED
    if hasattr(config, "workerinput") or _OWNER_LOCK_SWEEP_STARTED is not None:
        return
    # Filesystem timestamps are coarser than time.time(), so a file created
    # just after the session starts can carry a slightly earlier mtime.
    _OWNER_LOCK_SWEEP_STARTED = time.time() - 5.0


def _finish_owner_lock_sweep() -> None:
    global _OWNER_LOCK_SWEEP_STARTED
    started, _OWNER_LOCK_SWEEP_STARTED = _OWNER_LOCK_SWEEP_STARTED, None
    if started is None or os.name == "nt":
        # Exclusion is decided by share modes at open time on Windows, so
        # probing a live lock would itself fail rather than report the holder.
        return
    try:
        root = project_module._owner_lock_root()
    except Exception:
        return
    try:
        sweep_owner_locks(root, started)
    except Exception:
        # A session that already passed must not fail in teardown over it.
        return


_CUSTODY_RECORDS_DIR = None
_CUSTODY_PREVIOUS_RECORDS = None
_CUSTODY_REPORT = None


def _select_custody_engine(config):
    global _CUSTODY_RECORDS_DIR, _CUSTODY_PREVIOUS_RECORDS
    from tests import custody_engine_compare as custody_compare
    mode = os.environ.get(custody_compare.MODE_ENV, "").strip().lower()
    if not mode:
        return
    if mode not in {"shadow", "census"}:
        raise pytest.UsageError("KASSIBER_TEST_CUSTODY_ENGINE must be shadow or census")
    if not hasattr(config, "workerinput"):
        _CUSTODY_RECORDS_DIR = tempfile.TemporaryDirectory(prefix="kassiber-custody-census-")
        records = Path(_CUSTODY_RECORDS_DIR.name) / "records.jsonl"
        records.touch()
        _CUSTODY_PREVIOUS_RECORDS = os.environ.get(custody_compare.RECORDS_ENV)
        os.environ[custody_compare.RECORDS_ENV] = str(records)
    custody_compare.install_custody_mode(mode)
    _arm_child_hooks()


def _finish_custody_session(session):
    global _CUSTODY_REPORT
    if _CUSTODY_RECORDS_DIR is None:
        return
    from tests import custody_engine_compare as compare
    _CUSTODY_REPORT = compare.session_report(compare.read_records(os.environ[compare.RECORDS_ENV]))
    if os.environ.get("KASSIBER_TEST_CUSTODY_LOADER_BIN"):
        loader_records = Path(os.environ[compare.RECORDS_ENV] + ".loader")
        count = len(loader_records.read_text().splitlines()) if loader_records.exists() else 0
        _CUSTODY_REPORT[0].append(f"native loader: {count} exact v1 byte matches")
    if _CUSTODY_REPORT[1] and session.exitstatus in (pytest.ExitCode.OK, pytest.ExitCode.NO_TESTS_COLLECTED):
        session.exitstatus = pytest.ExitCode.TESTS_FAILED


def _custody_terminal_summary(reporter):
    if _CUSTODY_REPORT is None:
        return
    reporter.write_sep("-", "custody engine " + os.environ.get("KASSIBER_TEST_CUSTODY_ENGINE", ""))
    for line in _CUSTODY_REPORT[0]:
        reporter.write_line(line)
    for line in _CUSTODY_REPORT[1]:
        reporter.write_line("FAILED: " + line, red=True)


def _unconfigure_custody():
    global _CUSTODY_RECORDS_DIR
    from tests import custody_engine_compare as compare
    compare.uninstall_custody_mode()
    if _CUSTODY_RECORDS_DIR is not None:
        if _CUSTODY_PREVIOUS_RECORDS is None:
            os.environ.pop(compare.RECORDS_ENV, None)
        else:
            os.environ[compare.RECORDS_ENV] = _CUSTODY_PREVIOUS_RECORDS
        _CUSTODY_RECORDS_DIR.cleanup()
        _CUSTODY_RECORDS_DIR = None
