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

import pytest

from kassiber.operator import project as project_module


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
