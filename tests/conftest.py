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


_EGRESS_STACK: contextlib.ExitStack | None = None
_TAX_ENGINE_STACK: contextlib.ExitStack | None = None
_PREVIOUS_TEST_NO_EGRESS: str | None = None
_PREVIOUS_PYTHONPATH: str | None = None
_PREVIOUS_OFFLINE_PREFERENCE: str | None = None
_OFFLINE_PREFERENCE_DIR: tempfile.TemporaryDirectory | None = None


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


def _select_tax_engine_backend() -> None:
    """Route the session's tax calculations per `KASSIBER_TEST_TAX_ENGINE`.

    `rp2` or `native` runs every calculation on that backend; `shadow` runs
    each `GenericRP2TaxEngine.build_ledger_state` call on both and fails on
    any semantic difference (`tests/tax_engine_compare.py`). Unset keeps the
    product default. Product code reads no such variable, so subprocesses
    such as the smoke tests' daemon keep the default.
    """
    global _TAX_ENGINE_STACK
    mode = os.environ.get("KASSIBER_TEST_TAX_ENGINE", "").strip().lower()
    if not mode or _TAX_ENGINE_STACK is not None:
        return
    from kassiber.core.engines.rp2 import _use_tax_engine_backend
    from tests.tax_engine_compare import shadow_build_ledger_state

    stack = contextlib.ExitStack()
    if mode in ("rp2", "native"):
        stack.enter_context(_use_tax_engine_backend(mode))
    elif mode == "shadow":
        stack.enter_context(shadow_build_ledger_state())
    else:
        raise pytest.UsageError(
            f"KASSIBER_TEST_TAX_ENGINE must be rp2, native, or shadow, not {mode!r}"
        )
    _TAX_ENGINE_STACK = stack


def pytest_configure(config: pytest.Config) -> None:
    """Fail any test that tries to leave the machine.

    The suite's other no-network tests assert `not_called()` on one patched
    function each, so they only notice the path they were written for. This
    catches the ones nobody wrote a test for -- including egress that skips
    the ledger entirely, which is how the release check went unrecorded.

    `PYTHONPATH` carries it into the daemon subprocess the smoke tests spawn:
    `Popen` runs without `env=`, so the child inherits the variable and `site`
    imports `tests/_egress_guard/sitecustomize.py` before the daemon starts.

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
    _isolate_offline_preference()
    _select_tax_engine_backend()
    if env_flag("KASSIBER_INTEGRATION"):
        return

    global _EGRESS_STACK, _PREVIOUS_TEST_NO_EGRESS, _PREVIOUS_PYTHONPATH
    if _EGRESS_STACK is not None:
        return

    root = Path(__file__).resolve().parent.parent
    _PREVIOUS_TEST_NO_EGRESS = os.environ.get("KASSIBER_TEST_NO_EGRESS")
    _PREVIOUS_PYTHONPATH = os.environ.get("PYTHONPATH")
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    os.environ["PYTHONPATH"] = os.pathsep.join(
        [
            str(root / "tests" / "_egress_guard"),
            str(root),
            *([_PREVIOUS_PYTHONPATH] if _PREVIOUS_PYTHONPATH else []),
        ]
    )
    _EGRESS_STACK = contextlib.ExitStack()
    _EGRESS_STACK.enter_context(no_egress_guard(enabled=True))


def pytest_unconfigure(config: pytest.Config) -> None:
    global _EGRESS_STACK, _OFFLINE_PREFERENCE_DIR, _TAX_ENGINE_STACK
    if _TAX_ENGINE_STACK is not None:
        _TAX_ENGINE_STACK.close()
        _TAX_ENGINE_STACK = None
    if _OFFLINE_PREFERENCE_DIR is not None:
        if _PREVIOUS_OFFLINE_PREFERENCE is None:
            os.environ.pop("KASSIBER_OFFLINE_PREFERENCE_FILE", None)
        else:
            os.environ["KASSIBER_OFFLINE_PREFERENCE_FILE"] = _PREVIOUS_OFFLINE_PREFERENCE
        _OFFLINE_PREFERENCE_DIR.cleanup()
        _OFFLINE_PREFERENCE_DIR = None
    if _EGRESS_STACK is None:
        return
    _EGRESS_STACK.close()
    _EGRESS_STACK = None
    if _PREVIOUS_TEST_NO_EGRESS is None:
        os.environ.pop("KASSIBER_TEST_NO_EGRESS", None)
    else:
        os.environ["KASSIBER_TEST_NO_EGRESS"] = _PREVIOUS_TEST_NO_EGRESS
    if _PREVIOUS_PYTHONPATH is None:
        os.environ.pop("PYTHONPATH", None)
    else:
        os.environ["PYTHONPATH"] = _PREVIOUS_PYTHONPATH


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
