"""Launch the real operator server without depending on the host's logind state.

The subprocess integration suite supplies an isolated, owner-only
``XDG_RUNTIME_DIR`` and exercises the server's real IPC and process lifecycle.
Hosted CI runners are not interactive login sessions, so their logind state is
not a meaningful input to those tests.  Production entrypoints never import
this fixture and continue to fail closed when neither logout-lifetime primitive
is available.
"""

from __future__ import annotations

from kassiber.operator import server


def main() -> int:
    server._linux_logind_user_alive = lambda: None
    return server.main()


if __name__ == "__main__":
    raise SystemExit(main())
