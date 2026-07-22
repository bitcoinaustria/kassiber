"""Launch the real operator server without depending on the host's logind state.

The subprocess integration suite supplies an isolated, owner-only
``XDG_RUNTIME_DIR`` and exercises the server's real IPC and process lifecycle.
Hosted CI runners are not interactive login sessions, so their logind state is
not a meaningful input to those tests.  Production entrypoints never import
this fixture and continue to fail closed when neither logout-lifetime primitive
is available.
"""

from __future__ import annotations

import os
import stat
from pathlib import Path

from kassiber.operator import server
from kassiber.operator.protocol import TEST_RUNTIME_OVERRIDE_ENV


def _isolated_test_runtime_is_trusted(root: Path, info: os.stat_result) -> bool:
    """Trust only the integration suite's explicitly gated private runtime."""

    configured = os.environ.get("KASSIBER_OPERATOR_RUNTIME_DIR")
    if not configured or os.environ.get(TEST_RUNTIME_OVERRIDE_ENV) != "1":
        return False
    try:
        expected = Path(configured).resolve(strict=True)
    except OSError:
        return False
    return (
        root == expected
        and stat.S_ISDIR(info.st_mode)
        and (not hasattr(os, "getuid") or info.st_uid == os.getuid())
        and stat.S_IMODE(info.st_mode) == 0o700
    )


def main() -> int:
    server._linux_logind_user_alive = lambda: None
    server._login_session_runtime_path_is_trusted = (
        _isolated_test_runtime_is_trusted
    )
    return server.main()


if __name__ == "__main__":
    raise SystemExit(main())
