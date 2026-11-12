"""Fail closed when a Python test child would run without the no-egress guard.

The test session arms its socket guard in Python children through
`tests/_child_hooks/sitecustomize.py`, which `site` imports only when that
directory is on the child's `PYTHONPATH`, the child keeps
`KASSIBER_TEST_NO_EGRESS`, and its interpreter honours both (no `-I`, `-S` or
`-E`). An environment that replaces `PYTHONPATH` silently drops the guard:
on 2026-11-08 a parity fixture did, and a Python reference child reached the
built-in public Electrum default. Two checks keep that from recurring:

- `require_guarded_child` (runner side): `tests/cli_runner.py` and
  `runtime_parity.child_env` refuse to start a Python child the guard would
  not reach while the session is armed.
- `assert_guard_active` (child side): `tests/__init__.py` runs it in every
  `python -m tests.<module>` child, so a reference started with a bare
  `subprocess.run` exits 78 before running unguarded. The hook flags its own
  import of `tests` (`_ARMING`), which is the one import made before the
  guard exists.

Integration lanes that leave `KASSIBER_TEST_NO_EGRESS` unset are not armed and
are not checked; `KASSIBER_NO_EGRESS` is the product's own kill switch.
"""

from __future__ import annotations

import os
from collections.abc import Mapping, Sequence
from pathlib import Path
import sys

SWITCH = "KASSIBER_TEST_NO_EGRESS"
CHILD_HOOKS = Path(__file__).resolve().parent / "_child_hooks"
UNGUARDED_EXIT = 78


class UnguardedChildError(RuntimeError):
    """A Python test child would run without the session's no-egress guard."""


def armed(env: Mapping[str, str]) -> bool:
    """Whether `env` arms the test guard (any non-empty value, as the hook reads it)."""
    return bool(env.get(SWITCH))


def _sitecustomize_dir(pythonpath: str) -> Path | None:
    """The first `PYTHONPATH` entry that supplies `sitecustomize`."""
    for entry in pythonpath.split(os.pathsep):
        if entry and (Path(entry) / "sitecustomize.py").is_file():
            return Path(entry).resolve()
    return None


def _disables_site(option: str) -> bool:
    """`-I` (isolated), `-S` (no site) or `-E` (ignore PYTHON*), alone or combined."""
    if not option.startswith("-") or option.startswith("--") or option[1:2] in {"W", "X", "c", "m"}:
        return False
    return bool(set(option[1:]) & {"I", "S", "E"})


def require_guarded_child(
    env: Mapping[str, str] | None = None,
    python_options: Sequence[str] = (),
) -> None:
    """Refuse a Python child `env` that would drop the armed session's guard."""
    if not armed(os.environ):
        return
    env = os.environ if env is None else env
    problems = []
    if not armed(env):
        problems.append(f"its environment drops {SWITCH}")
    hooks = _sitecustomize_dir(env.get("PYTHONPATH", ""))
    if hooks != CHILD_HOOKS:
        problems.append(
            f"its PYTHONPATH does not load the guard from {CHILD_HOOKS}"
            + (f" (sitecustomize resolves to {hooks})" if hooks else "")
        )
    disabling = [option for option in python_options if _disables_site(option)]
    if disabling:
        problems.append(f"interpreter options {disabling} skip sitecustomize")
    if problems:
        raise UnguardedChildError(
            "refusing to start a Python test child without the no-egress guard: "
            + "; ".join(problems)
            + ". Copy os.environ and extend PYTHONPATH instead of replacing it."
        )


def guarded_pythonpath(*entries: str | os.PathLike[str]) -> str:
    """A child `PYTHONPATH`: the guard hook, the repository, then `entries`."""
    root = CHILD_HOOKS.parent.parent
    return os.pathsep.join([str(CHILD_HOOKS), str(root), *(os.fspath(entry) for entry in entries)])


def guard_active() -> bool:
    """Whether this process runs the child hook's live socket guard."""
    hook = sys.modules.get("sitecustomize")
    path = getattr(hook, "__file__", None)
    return (
        path is not None
        and Path(path).resolve().parent == CHILD_HOOKS
        and getattr(hook, "_GUARD", None) is not None
    )


def assert_guard_active() -> None:
    """Child side: exit before running when armed but unguarded."""
    if armed(os.environ) and not guard_active():
        sys.stderr.write(
            f"Kassiber test child refused to run: {SWITCH} is set but the no-egress guard is not "
            f"active (tests/_child_hooks is not this child's sitecustomize). Keep {CHILD_HOOKS} "
            "first on PYTHONPATH.\n"
        )
        sys.stderr.flush()
        # Not SystemExit: raised while runpy resolves `-m`, it would become a
        # traceback and exit status 1.
        os._exit(UNGUARDED_EXIT)
