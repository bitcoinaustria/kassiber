"""Arm the no-egress guard inside Python child processes.

The daemon smoke tests spawn a real `python -m kassiber daemon`. An in-process
monkeypatch cannot reach it, but `Popen` is called without `env=`, so the child
inherits the guard variable, and `site` imports this module before the daemon
runs. That makes the child guarded too, which is where most of the interesting
egress lives.

`KASSIBER_TEST_NO_EGRESS` is what the suite sets: it arms the socket guard
without telling product code anything. `KASSIBER_NO_EGRESS` also arms it, but
that one is a product kill switch the chain observers read directly.

Only Python children are covered. The Node AI broker and `lightning-cli`
inherit the variable but nothing enforces it for them.
"""

from __future__ import annotations

import os

# Module-global on purpose. Without a live reference the context manager is
# garbage-collected, its `finally` runs, and the guard silently uninstalls
# itself -- leaving a process that looks armed and blocks nothing.
_GUARD = None

if os.environ.get("KASSIBER_TEST_NO_EGRESS") or os.environ.get(
    "KASSIBER_NO_EGRESS"
):
    try:
        from tests.integration.env import no_egress_guard

        _GUARD = no_egress_guard(enabled=True)
        _GUARD.__enter__()
    except Exception:
        # A child that cannot arm the guard must still start; the parent
        # process guard remains the primary check.
        _GUARD = None
