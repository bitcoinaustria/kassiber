"""Executable selection for source installs and frozen sidecars."""

from __future__ import annotations

import sys
from collections.abc import MutableMapping


def broker_server_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable, "--operator-broker-server"]
    return [sys.executable, "-m", "kassiber.operator.server"]


def cli_child_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable]
    return [sys.executable, "-m", "kassiber"]


def prepare_independent_child_environment(environment: MutableMapping[str, str]) -> None:
    """Make a re-executed one-file build unpack into its own runtime directory."""

    if getattr(sys, "frozen", False):
        environment["PYINSTALLER_RESET_ENVIRONMENT"] = "1"
