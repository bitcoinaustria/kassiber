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

import os
import time
from pathlib import Path

import pytest

from kassiber.operator import project as project_module
from tests.integration.env import env_flag, no_egress_guard


@pytest.fixture(autouse=True, scope="session")
def _block_egress():
    """Fail any test that tries to leave the machine.

    The suite's other no-network tests assert `not_called()` on one patched
    function each, so they only notice the path they were written for. This
    catches the ones nobody wrote a test for -- including egress that skips
    the ledger entirely, which is how the release check went unrecorded.

    `PYTHONPATH` carries it into the daemon subprocess the smoke tests spawn:
    `Popen` runs without `env=`, so the child inherits `KASSIBER_NO_EGRESS`
    and `site` imports `tests/_egress_guard/sitecustomize.py` before the
    daemon starts.

    Loopback stays allowed: ~36 daemon smoke tests bind local
    `ThreadingHTTPServer` fakes. So the invariant's "including loopback
    service/provider probes" clause is *not* covered here -- a test that
    needs that asserts it directly.
    """
    if env_flag("KASSIBER_INTEGRATION") or env_flag("KASSIBER_MEDIUM"):
        yield
        return

    root = Path(__file__).resolve().parent.parent
    previous_path = os.environ.get("PYTHONPATH")
    os.environ["KASSIBER_NO_EGRESS"] = "1"
    os.environ["PYTHONPATH"] = os.pathsep.join(
        [
            str(root / "tests" / "_egress_guard"),
            str(root),
            *([previous_path] if previous_path else []),
        ]
    )
    try:
        with no_egress_guard(enabled=True):
            yield
    finally:
        os.environ.pop("KASSIBER_NO_EGRESS", None)
        if previous_path is None:
            os.environ.pop("PYTHONPATH", None)
        else:
            os.environ["PYTHONPATH"] = previous_path


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
