"""Executable selection for source installs and frozen sidecars."""

from __future__ import annotations

import sys


def broker_server_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable, "--operator-broker-server"]
    return [sys.executable, "-m", "kassiber.operator.server"]


def cli_child_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable]
    return [sys.executable, "-m", "kassiber"]
