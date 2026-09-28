"""Executable selection for source installs and frozen sidecars."""

from __future__ import annotations

import os
import sys
from collections.abc import MutableMapping
from pathlib import Path


def broker_server_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable, "--operator-broker-server"]
    return [sys.executable, "-m", "kassiber.operator.server"]


def cli_child_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable]
    return [sys.executable, "-m", "kassiber"]


def trusted_launch_directory() -> str:
    """Where re-executed broker and worker processes start.

    `python -m` puts its start directory first on `sys.path`, and workers
    inherit the lease passphrase pipe before any Kassiber code can run. Start
    where this `kassiber` package was imported from, never in a client's
    directory, so an agent repository cannot shadow the package.
    """

    if getattr(sys, "frozen", False):
        return os.path.dirname(os.path.abspath(sys.executable))
    import kassiber

    return str(Path(kassiber.__file__).resolve().parent.parent)


def prepare_independent_child_environment(environment: MutableMapping[str, str]) -> None:
    """Make a re-executed one-file build unpack into its own runtime directory."""

    if getattr(sys, "frozen", False):
        environment["PYINSTALLER_RESET_ENVIRONMENT"] = "1"
