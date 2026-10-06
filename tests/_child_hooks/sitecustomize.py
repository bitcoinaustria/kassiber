"""Carry the test session's guards into Python child processes.

The daemon smoke tests spawn a real `python -m kassiber daemon`, and the
review regressions run `python -m kassiber journals process`. An in-process
monkeypatch cannot reach them, but `Popen` is called without `env=` (or with
a copy of `os.environ`), so the child inherits the session's variables and
`site` imports this module before the child runs.

The no-egress guard: `KASSIBER_TEST_NO_EGRESS` is what the suite sets; it
arms the socket guard without telling product code anything.
`KASSIBER_NO_EGRESS` also arms it, but that one is a product kill switch the
chain observers read directly.

The tax engine mode: under `KASSIBER_TEST_TAX_ENGINE=shadow` or `census` the
child installs the same patch as the session (`tests/tax_engine_compare.py`)
once `kassiber.core.engines` has loaded, or once the harness has, when the
child imports it first; so its ledger builds are compared with the oracle or
counted too, and their route records reach the session. A child that never
loads the engines imports nothing extra.

Only Python children are covered. The Node AI broker and `lightning-cli`
inherit the variables but nothing enforces them there.
"""

from __future__ import annotations

import importlib.abc
import os
import sys

# Module-global on purpose. Without a live reference the context manager is
# garbage-collected, its `finally` runs, and the guard silently uninstalls
# itself -- leaving a process that looks armed and blocks nothing.
_GUARD = None

_EGRESS_MODE = os.environ.get("KASSIBER_TEST_NO_EGRESS") or os.environ.get(
    "KASSIBER_NO_EGRESS"
)

if _EGRESS_MODE:
    try:
        from tests.integration.env import no_egress_guard

        # `strict` also blocks loopback, for the launch-path regression that
        # pins the invariant's "including loopback probes" clause.
        _GUARD = no_egress_guard(
            enabled=True, allow_loopback=_EGRESS_MODE.strip().lower() != "strict"
        )
        _GUARD.__enter__()
    except Exception as exc:
        print("Kassiber test no-egress guard could not start", file=sys.stderr)
        raise SystemExit(78) from exc


_TAX_MODE = os.environ.get("KASSIBER_TEST_TAX_ENGINE", "").strip().lower()
_TAX_ENGINES_PACKAGE = "kassiber.core.engines"
_TAX_HARNESS = "tests.tax_engine_compare"


def _install_tax_mode(harness=None) -> None:
    try:
        if harness is None:
            from tests import tax_engine_compare as harness

        harness.install_tax_mode(_TAX_MODE)
    except Exception as exc:
        print("Kassiber test tax engine mode could not start", file=sys.stderr)
        raise SystemExit(78) from exc


def _engines_loaded(_module) -> None:
    if _TAX_HARNESS in sys.modules:
        # The harness itself is loading the engines; the mode is installed
        # once the harness has finished (`_harness_loaded`).
        return
    _install_tax_mode()


def _harness_loaded(module) -> None:
    # Installing is idempotent, so a pytest child's `conftest.py` installing
    # the same mode afterwards changes nothing.
    _install_tax_mode(module)


class _InstallTaxModeAfterImport(importlib.abc.MetaPathFinder):
    """Install the tax mode right after the engines package or the harness executes.

    Installing it here instead of importing the harness up front keeps the
    child's own imports in their usual order; by the time the engines
    package has run, the harness can import everything it needs. A child
    that imports the harness first (which imports the engines) gets the
    mode once the harness has finished, not halfway through it.
    """

    _AFTER = {_TAX_ENGINES_PACKAGE: _engines_loaded, _TAX_HARNESS: _harness_loaded}

    def __init__(self) -> None:
        self._found: set[str] = set()

    def find_spec(self, name, path=None, target=None):
        after = self._AFTER.get(name)
        if after is None or name in self._found:
            return None
        self._found.add(name)
        for finder in sys.meta_path:
            find_spec = getattr(finder, "find_spec", None)
            if finder is self or find_spec is None:
                continue
            spec = find_spec(name, path, target)
            if spec is not None and spec.loader is not None:
                break
        else:
            return None
        exec_module = spec.loader.exec_module

        def exec_then_install(module) -> None:
            exec_module(module)
            after(module)

        spec.loader.exec_module = exec_then_install
        return spec


if _TAX_MODE in {"shadow", "census"}:
    sys.meta_path.insert(0, _InstallTaxModeAfterImport())
