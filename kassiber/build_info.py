from __future__ import annotations

import json
from importlib import resources
from typing import Any

from . import __version__


def packaged_build_info() -> dict[str, Any]:
    """Return optional metadata embedded by the binary packaging jobs."""

    try:
        payload = (
            resources.files("kassiber")
            .joinpath("data")
            .joinpath("BUILD_INFO.json")
            .read_text(encoding="utf-8")
        )
        decoded = json.loads(payload)
    except (FileNotFoundError, ModuleNotFoundError, OSError, TypeError, json.JSONDecodeError):
        return {}
    if not isinstance(decoded, dict):
        return {}
    return decoded


def version_text() -> str:
    info = packaged_build_info()
    version = str(info.get("version") or __version__)
    details: list[str] = []
    channel = str(info.get("channel") or "").strip()
    commit = str(info.get("commit") or "").strip()
    if channel:
        details.append(channel)
    if commit:
        details.append(f"commit {commit[:12]}")
    suffix = f" ({', '.join(details)})" if details else ""
    return f"Kassiber {version}{suffix}"
