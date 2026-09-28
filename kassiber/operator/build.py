"""Which Kassiber build a broker or client is, without revealing paths.

Every build of one OS user shares the single per-user broker endpoint, and a
broker runs its own build's code for every queued child. A passphrase handed
to `operator unlock` should therefore reach only the same build. This is an
integrity guard against accidental mixing (a dev worktree's broker serving the
release app), not a boundary against same-user code, which can report any
identity it likes.
"""

from __future__ import annotations

import hashlib
import os
import sys
from pathlib import Path
from typing import Any

from .. import __version__
from ..build_info import packaged_build_info


def build_identity() -> dict[str, Any]:
    info = packaged_build_info()
    frozen = bool(getattr(sys, "frozen", False))
    # One-file sidecars unpack into a fresh temp dir per process, so the
    # stable location of a frozen build is its executable; source installs
    # are distinguished by the package directory they import from.
    if frozen:
        location = _frozen_location()
    else:
        import kassiber

        location = str(Path(kassiber.__file__).resolve().parent)
    commit = str(info.get("commit") or "").strip()
    channel = str(info.get("channel") or "").strip()
    return {
        "version": str(info.get("version") or __version__),
        "channel": channel or None,
        "commit": commit[:12] or None,
        "frozen": frozen,
        # A short digest keeps worktrees and installs apart without exposing
        # a local path to agents that read status output.
        "origin": hashlib.sha256(location.encode("utf-8")).hexdigest()[:12],
    }


def _frozen_location() -> str:
    executable = Path(sys.executable).resolve()
    # An AppImage mounts itself at a fresh temporary directory on every
    # launch, so its sidecar's path differs each time; the AppImage file is
    # the stable artifact. Only trust APPIMAGE for an executable inside this
    # mount (APPDIR), not one merely inheriting the variable.
    appimage = os.environ.get("APPIMAGE")
    appdir = os.environ.get("APPDIR")
    if appimage and appdir:
        try:
            executable.relative_to(Path(appdir).resolve())
        except ValueError:
            pass
        else:
            return str(Path(appimage).resolve())
    return str(executable)


def describe_build(build: Any) -> str:
    if not isinstance(build, dict):
        return "an older Kassiber build"
    version = build.get("version") or "unknown"
    kind = "packaged" if build.get("frozen") else "source"
    channel = f" {build['channel']}" if build.get("channel") else ""
    return f"Kassiber {version}{channel} ({kind}, {build.get('origin') or 'unknown origin'})"
