"""User-controlled access for external agents (MCP), off by default.

One small global preference beside the update-check consent, so the desktop
Settings switch and every `kassiber mcp` process enforce the same choice
before any book is opened. The desktop also mirrors its AI master switch
here: external agents are served only while both are on. Installs that never
ran the desktop record no master-switch decision (`None`), which does not
block a CLI user's own opt-in.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from .db import DEFAULT_CONFIG_DIRNAME
from .errors import AppError
from .projects import default_state_root
from .private_files import atomic_write_private, read_small_private_file


SCHEMA_VERSION = 1
FILENAME = "agent-access.json"
PATH_ENV = "KASSIBER_AGENT_ACCESS_FILE"
_MAX_BYTES = 1024


def preference_path() -> Path:
    override = os.environ.get(PATH_ENV)
    if override:
        return Path(override).expanduser()
    return default_state_root() / DEFAULT_CONFIG_DIRNAME / FILENAME


def _read(path: Path) -> tuple[bool, bool | None]:
    """Return (mcp_enabled, ai_features_enabled); anything unreadable is off."""

    raw = read_small_private_file(path, _MAX_BYTES)
    if raw is None:
        return False, None
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return False, None
    if (
        not isinstance(payload, dict)
        or type(payload.get("schema_version")) is not int
        or payload["schema_version"] != SCHEMA_VERSION
    ):
        return False, None
    mcp_enabled = payload.get("mcp_enabled")
    ai_features = payload.get("ai_features_enabled")
    return (
        mcp_enabled is True,
        ai_features if isinstance(ai_features, bool) else None,
    )


def agent_access_status(path: Path | None = None) -> dict[str, Any]:
    mcp_enabled, ai_features_enabled = _read(path or preference_path())
    reason = None
    if not mcp_enabled:
        reason = "mcp_disabled"
    elif ai_features_enabled is False:
        reason = "ai_features_disabled"
    return {
        "mcp_enabled": mcp_enabled,
        "ai_features_enabled": ai_features_enabled,
        "mcp_available": reason is None,
        "reason": reason,
    }


def set_agent_access(
    *,
    mcp_enabled: bool | None = None,
    ai_features_enabled: bool | None = None,
    path: Path | None = None,
) -> dict[str, Any]:
    """Update either field, keeping the other; the desktop and CLI share it."""

    destination = path or preference_path()
    current_mcp, current_ai = _read(destination)
    document = {
        "schema_version": SCHEMA_VERSION,
        "mcp_enabled": current_mcp if mcp_enabled is None else bool(mcp_enabled),
        "ai_features_enabled": (
            current_ai if ai_features_enabled is None else bool(ai_features_enabled)
        ),
    }
    atomic_write_private(destination, json.dumps(document, sort_keys=True) + "\n")
    return agent_access_status(destination)


def require_mcp_access(path: Path | None = None) -> None:
    status = agent_access_status(path)
    if status["mcp_available"]:
        return
    if status["reason"] == "ai_features_disabled":
        raise AppError(
            "AI features are turned off in Kassiber",
            code="ai_features_disabled",
            hint="Ask the user to turn on AI features in Kassiber Settings > AI.",
            retryable=False,
        )
    raise AppError(
        "External agents are not enabled for Kassiber",
        code="mcp_disabled",
        hint=(
            "Ask the user to turn on External agents in Kassiber Settings > AI, "
            "or to run `kassiber mcp enable` in their own terminal."
        ),
        retryable=False,
    )
