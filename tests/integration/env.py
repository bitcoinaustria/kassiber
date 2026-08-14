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


class EgressBlocked(BaseException):
    """Raised when a guarded test process tries to leave the machine.

    Deliberately a `BaseException`: several product paths wrap any `Exception`
    into an `AppError` envelope, which would turn a guard trip into a plausible
    "backend unreachable" result and let the test pass.
    """


@contextlib.contextmanager
def no_egress_guard(
    *, enabled: bool | None = None, allow_loopback: bool = True
) -> Iterator[None]:
    """Block socket connects, datagrams, and DNS inside a test process.

    The guard is intentionally test-local: it proves fast/medium fixtures do not
    reach live exchanges or public backends without changing product runtime
    behavior. Loopback is allowed so daemon bridges, Docker-published regtest
    services, and local SQLite-adjacent helpers can still run.

    Two variables reach this, deliberately kept apart:

    - `KASSIBER_TEST_NO_EGRESS` arms only this socket guard. It is what the
      suite sets, because it says nothing to product code.
    - `KASSIBER_NO_EGRESS` is a product kill switch that the BDK and LWK
      observers read directly and honor destination-blind -- it refuses a
      loopback server too. It also arms this guard, so the integration
      harness keeps working by setting one variable.

    `allow_loopback=False` covers the invariant's "including loopback
    service/provider probes" clause, which the default cannot: the smoke tests
    serve local HTTP fakes, so a suite-wide loopback block would fail the tests
    that need one. Set `KASSIBER_TEST_NO_EGRESS=strict` to get it in a child
    process.

    Blind spots worth knowing: sockets opened inside native libraries (bdkpython,
    lwk) are invisible to a Python patch -- those honor `KASSIBER_NO_EGRESS`
    themselves -- and non-Python children (the Node AI broker, `lightning-cli`)
    inherit the variable but nothing enforces it for them.
    """

    active = (
        (env_flag("KASSIBER_TEST_NO_EGRESS") or env_flag("KASSIBER_NO_EGRESS"))
        if enabled is None
        else bool(enabled)
    )
    if not active:
        yield
        return

    original_connect = socket.socket.connect
    original_connect_ex = socket.socket.connect_ex
    original_sendto = socket.socket.sendto
    original_getaddrinfo = socket.getaddrinfo

    def _blocked_address(address: Any) -> str | None:
        if isinstance(address, tuple) and address:
            host = str(address[0])
            if not allow_loopback or not _is_loopback(host):
                return host
        return None

    def guarded_connect(self: socket.socket, address: Any):
        host = _blocked_address(address)
        if host is not None:
            raise EgressBlocked(f"no-egress guard blocked socket.connect to {host}")
        return original_connect(self, address)

    def guarded_connect_ex(self: socket.socket, address: Any):
        host = _blocked_address(address)
        if host is not None:
            raise EgressBlocked(
                f"no-egress guard blocked socket.connect_ex to {host}"
            )
        return original_connect_ex(self, address)

    def guarded_sendto(self: socket.socket, data: Any, *args: Any):
        address = args[-1] if args else None
        host = _blocked_address(address)
        if host is not None:
            raise EgressBlocked(f"no-egress guard blocked socket.sendto to {host}")
        return original_sendto(self, data, *args)

    def guarded_getaddrinfo(host: Any, *args: Any, **kwargs: Any):
        # Resolving a name is already a request to a nameserver, and it is the
        # first thing most egress paths do -- so blocking it here names the
        # offending host instead of failing later with a connect error.
        if host is not None and (not allow_loopback or not _is_loopback(str(host))):
            raise EgressBlocked(f"no-egress guard blocked DNS lookup for {host}")
        return original_getaddrinfo(host, *args, **kwargs)

    socket.socket.connect = guarded_connect
    socket.socket.connect_ex = guarded_connect_ex
    socket.socket.sendto = guarded_sendto
    socket.getaddrinfo = guarded_getaddrinfo
    try:
        yield
    finally:
        socket.socket.connect = original_connect
        socket.socket.connect_ex = original_connect_ex
        socket.socket.sendto = original_sendto
        socket.getaddrinfo = original_getaddrinfo


__all__ = [
    "EgressBlocked",
    "env_flag",
    "no_egress_guard",
    "skip_unless_env",
    "skip_unless_integration",
    "skip_unless_medium",
]
