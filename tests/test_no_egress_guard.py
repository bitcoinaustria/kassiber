"""Meta-tests for the suite-wide no-egress guard.

A guard that silently fails to install is worse than none, because every test
that follows it reports a clean run it never actually earned. These assert the
guard is armed in this process and in the Python children the daemon tests
spawn.
"""

from __future__ import annotations

import os
import socket
import subprocess
import sys

import pytest

from tests.integration.env import EgressBlocked


def test_guard_blocks_dns_in_this_process():
    with pytest.raises(EgressBlocked):
        socket.getaddrinfo("api.github.com", 443)


def test_guard_blocks_connect_in_this_process():
    probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        with pytest.raises(EgressBlocked):
            probe.connect(("93.184.216.34", 80))
    finally:
        probe.close()


def test_guard_does_not_block_loopback():
    # Daemon bridges and the local HTTP fakes in the smoke tests depend on this.
    assert socket.getaddrinfo("127.0.0.1", 0)


def test_guard_is_not_an_exception_subclass():
    """`except Exception` must not swallow it.

    Several product paths turn any `Exception` into an `AppError` envelope,
    which would render a blocked request as a plausible "backend unreachable"
    result and let the test pass.
    """
    assert issubclass(EgressBlocked, BaseException)
    assert not issubclass(EgressBlocked, Exception)


def test_guard_reaches_spawned_python_children():
    """The daemon smoke tests spawn a real `python -m kassiber daemon`.

    An in-process monkeypatch cannot reach it. `Popen` runs without `env=`, so
    the child inherits KASSIBER_NO_EGRESS and `site` imports our
    `sitecustomize` before anything else runs.
    """
    assert os.environ.get("KASSIBER_NO_EGRESS")

    completed = subprocess.run(
        [
            sys.executable,
            "-c",
            "import socket; socket.getaddrinfo('api.github.com', 443)",
        ],
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert completed.returncode != 0
    assert "KASSIBER_NO_EGRESS" in completed.stderr
