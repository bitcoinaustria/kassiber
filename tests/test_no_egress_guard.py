"""Meta-tests for the no-egress guard.

A guard that silently fails to install is worse than none, because everything
that runs under it reports a clean result it never actually earned. These pin
what it blocks, that it reaches spawned Python children, and that it cannot be
swallowed by a broad `except Exception`.

`tests/conftest.py` arms it for every non-integration run via
`KASSIBER_TEST_NO_EGRESS`; these tests install it explicitly so they assert the
guard's own behavior rather than the suite hook's.
"""

from __future__ import annotations

import os
import socket
import subprocess
import sys
from pathlib import Path

import pytest

from tests.integration.env import EgressBlocked, no_egress_guard

ROOT = Path(__file__).resolve().parent.parent


def test_guard_blocks_dns():
    with no_egress_guard(enabled=True):
        with pytest.raises(EgressBlocked):
            socket.getaddrinfo("api.github.com", 443)


def test_guard_blocks_connect():
    probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        with no_egress_guard(enabled=True):
            with pytest.raises(EgressBlocked):
                probe.connect(("93.184.216.34", 80))
    finally:
        probe.close()


def test_guard_blocks_udp_sendto():
    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        with no_egress_guard(enabled=True):
            with pytest.raises(EgressBlocked):
                probe.sendto(b"probe", ("192.0.2.1", 9))
    finally:
        probe.close()


def test_guard_allows_loopback():
    # Daemon bridges and the local HTTP fakes in the smoke tests depend on this.
    with no_egress_guard(enabled=True):
        assert socket.getaddrinfo("127.0.0.1", 0)


def test_guard_uninstalls_itself_on_exit():
    original = socket.getaddrinfo
    with no_egress_guard(enabled=True):
        assert socket.getaddrinfo is not original
    assert socket.getaddrinfo is original


def test_guard_is_not_an_exception_subclass():
    """`except Exception` must not swallow it.

    Several product paths turn any `Exception` into an `AppError` envelope,
    which would render a blocked request as a plausible "backend unreachable"
    result and let the caller carry on.
    """
    assert issubclass(EgressBlocked, BaseException)
    assert not issubclass(EgressBlocked, Exception)


def test_guard_reaches_spawned_python_children():
    """`sitecustomize` carries the guard into Python children.

    An in-process monkeypatch cannot reach a subprocess. A child that inherits
    the guard variable imports `tests/_egress_guard/sitecustomize.py` through
    `site` before anything else runs, which is what makes the guard reach the
    daemon children the smoke tests spawn.
    """
    env = dict(os.environ)
    env["KASSIBER_TEST_NO_EGRESS"] = "1"
    env["PYTHONPATH"] = os.pathsep.join(
        [str(ROOT / "tests" / "_egress_guard"), str(ROOT)]
    )

    completed = subprocess.run(
        [sys.executable, "-c", "import socket; socket.getaddrinfo('api.github.com', 443)"],
        capture_output=True,
        text=True,
        timeout=60,
        env=env,
    )

    assert completed.returncode != 0
    assert "no-egress guard blocked" in completed.stderr


def test_child_exits_if_guard_cannot_start(tmp_path: Path):
    """A broken child guard must not produce a false-green launch test."""
    shadow = tmp_path / "shadow"
    shadow.mkdir()
    (shadow / "tests.py").write_text("# blocks the real tests package\n")
    env = dict(os.environ)
    env["KASSIBER_TEST_NO_EGRESS"] = "1"
    env["PYTHONPATH"] = os.pathsep.join(
        [str(ROOT / "tests" / "_egress_guard"), str(shadow)]
    )

    completed = subprocess.run(
        [sys.executable, "-c", "print('unguarded child ran')"],
        capture_output=True,
        text=True,
        timeout=60,
        cwd=tmp_path,
        env=env,
    )

    assert completed.returncode != 0
    assert "unguarded child ran" not in completed.stdout
    assert "Kassiber test no-egress guard could not start" in completed.stderr
