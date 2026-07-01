from __future__ import annotations

import contextlib
import ipaddress
import os
import socket
import unittest
from collections.abc import Iterator
from typing import Any


TRUE_VALUES = {"1", "true", "yes", "on"}


def env_flag(name: str) -> bool:
    return str(os.environ.get(name, "")).strip().lower() in TRUE_VALUES


def skip_unless_env(name: str, reason: str):
    return unittest.skipUnless(env_flag(name), f"{name}=1 required: {reason}")


skip_unless_integration = skip_unless_env(
    "KASSIBER_INTEGRATION",
    "slow live-node integration lane is opt-in",
)
skip_unless_medium = skip_unless_env(
    "KASSIBER_MEDIUM",
    "medium deterministic scenario lane is opt-in",
)


def _is_loopback(host: str) -> bool:
    if not host:
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return host in {"localhost", "ip6-localhost", "ip6-loopback"}


@contextlib.contextmanager
def no_egress_guard(*, enabled: bool | None = None) -> Iterator[None]:
    """Block non-loopback socket connects inside a test process.

    The guard is intentionally test-local: it proves fast/medium fixtures do not
    reach live exchanges or public backends without changing product runtime
    behavior. Loopback is allowed so daemon bridges, Docker-published regtest
    services, and local SQLite-adjacent helpers can still run.
    """

    active = env_flag("KASSIBER_NO_EGRESS") if enabled is None else bool(enabled)
    if not active:
        yield
        return

    original_connect = socket.socket.connect
    original_connect_ex = socket.socket.connect_ex

    def _blocked_address(address: Any) -> str | None:
        if isinstance(address, tuple) and address:
            host = str(address[0])
            if not _is_loopback(host):
                return host
        return None

    def guarded_connect(self: socket.socket, address: Any):
        host = _blocked_address(address)
        if host is not None:
            raise AssertionError(f"KASSIBER_NO_EGRESS blocked socket.connect to {host}")
        return original_connect(self, address)

    def guarded_connect_ex(self: socket.socket, address: Any):
        host = _blocked_address(address)
        if host is not None:
            raise AssertionError(f"KASSIBER_NO_EGRESS blocked socket.connect_ex to {host}")
        return original_connect_ex(self, address)

    socket.socket.connect = guarded_connect
    socket.socket.connect_ex = guarded_connect_ex
    try:
        yield
    finally:
        socket.socket.connect = original_connect
        socket.socket.connect_ex = original_connect_ex


__all__ = [
    "env_flag",
    "no_egress_guard",
    "skip_unless_env",
    "skip_unless_integration",
    "skip_unless_medium",
]
