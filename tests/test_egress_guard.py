"""A Python test child never runs without the session's no-egress guard.

Regression for 2026-11-08: a parity fixture replaced `PYTHONPATH`, which
dropped `tests/_child_hooks`, and its Python reference child reached the
built-in public Electrum default. Now the runner refuses such a child before
spawning it, and a `python -m tests.<module>` child refuses to run unguarded.
"""

import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests import cli_runner
from tests.egress_guard import CHILD_HOOKS, SWITCH, UNGUARDED_EXIT, UnguardedChildError, require_guarded_child
from tests.integration.runtime_parity import child_env

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture
def armed(monkeypatch):
    monkeypatch.setenv(SWITCH, "1")
    monkeypatch.setenv("PYTHONPATH", os.pathsep.join([str(CHILD_HOOKS), str(ROOT)]))


@pytest.fixture
def no_spawn(monkeypatch):
    def refuse(*args, **kwargs):
        raise AssertionError(f"spawned an unguarded child: {args}")

    monkeypatch.setattr(subprocess, "run", refuse)
    monkeypatch.setattr(subprocess, "Popen", refuse)


def _replaced(env, tmp_path):
    env["PYTHONPATH"] = str(ROOT)


def _emptied(env, tmp_path):
    env["PYTHONPATH"] = ""


def _shadowed(env, tmp_path):
    # Another sitecustomize earlier on the path wins over the hook.
    (tmp_path / "sitecustomize.py").write_text("")
    env["PYTHONPATH"] = os.pathsep.join([str(tmp_path), str(CHILD_HOOKS)])


def _switch_dropped(env, tmp_path):
    env[SWITCH] = ""


@pytest.mark.parametrize("change", [_replaced, _emptied, _shadowed, _switch_dropped],
                         ids=["replaced", "emptied", "shadowed", "switch-dropped"])
@pytest.mark.parametrize("launch", ["run", "popen"])
def test_runner_refuses_a_fixture_that_drops_the_guard(armed, no_spawn, tmp_path, change, launch):
    env = {**os.environ}
    change(env, tmp_path)
    with pytest.raises(UnguardedChildError, match="no-egress guard"):
        getattr(cli_runner, launch)(["--version"], reference=True, env=env)


@pytest.mark.parametrize("options", [("-I",), ("-S",), ("-E",), ("-IS",)])
def test_runner_refuses_interpreter_options_that_skip_the_hook(armed, no_spawn, options):
    with pytest.raises(UnguardedChildError, match="skip sitecustomize"):
        cli_runner.run(["--version"], reference=True, python_options=options)


def test_runtime_parity_env_is_guarded_until_a_caller_replaces_its_path(armed, no_spawn, tmp_path):
    env = child_env(tmp_path / "home", python_child=True)
    require_guarded_child(env)
    env["PYTHONPATH"] = str(ROOT)
    with pytest.raises(UnguardedChildError):
        cli_runner.run(["--version"], reference=True, env=env)


def test_guarded_and_unarmed_children_are_admitted(armed, monkeypatch):
    require_guarded_child({**os.environ})
    require_guarded_child({**os.environ}, ("-X", "dev", "-W", "error"))
    # An integration lane that never armed the switch is not checked.
    monkeypatch.delenv(SWITCH)
    require_guarded_child({"PYTHONPATH": str(ROOT)}, ("-I",))


def test_reference_child_without_the_hook_exits_before_running(armed, tmp_path):
    """The child-side check catches a bare `subprocess.run`, the original slip."""
    unguarded = subprocess.run(
        [sys.executable, "-m", "tests.egress_guard"],
        env={**os.environ, "PYTHONPATH": str(ROOT)}, capture_output=True, text=True, cwd=ROOT, timeout=60,
    )
    assert unguarded.returncode == UNGUARDED_EXIT, unguarded
    assert "no-egress guard is not active" in unguarded.stderr
    guarded = subprocess.run(
        [sys.executable, "-m", "tests.egress_guard"],
        env={**os.environ}, capture_output=True, text=True, cwd=tmp_path, timeout=60,
    )
    assert (guarded.returncode, guarded.stderr) == (0, ""), guarded


def test_the_2026_11_08_fixture_now_fails_instead_of_running_unguarded(armed, tmp_path):
    """The exact slip: a copied environment whose PYTHONPATH was replaced,
    starting the journal CLI reference with a bare subprocess call."""
    env = {**os.environ, "PYTHONPATH": str(ROOT)}
    result = subprocess.run(
        [sys.executable, "-m", "tests.rust_port.journal_cli_reference", "--machine", "--data-root",
         str(tmp_path / "data"), "wallets", "identify", "--txid", "f" * 64, "--verify-on-chain"],
        env=env, capture_output=True, text=True, cwd=ROOT, timeout=60,
    )
    assert result.returncode == UNGUARDED_EXIT, result
    assert result.stdout == ""
    assert "no-egress guard is not active" in result.stderr
    assert not (tmp_path / "data").exists()
