"""User-controlled access for external agents (MCP), off by default.

One small global preference beside the update-check consent, so the desktop
Settings switch and every `kassiber mcp` process enforce the same choice
before any book is opened. The desktop also mirrors its AI master switch
here: external agents are served only while both are on. Installs that never
ran the desktop record no master-switch decision (`None`), which does not
block a CLI user's own opt-in.

Besides the switch, the user chooses what an agent may do:

- `grant` is one of the operator broker's cumulative non-admin capabilities
  (`read`, `operator`, `accounting_decisions`); admin is never grantable;
- `agent_access` has the per-provider meaning (`full` or `restricted`).

A file written before these fields existed keeps its earlier behaviour,
`read` + `restricted`, and so does any value this build does not recognise.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .ai.providers import (
    AI_AGENT_ACCESS_FULL,
    AI_AGENT_ACCESS_MODES,
    AI_AGENT_ACCESS_RESTRICTED,
)
from .db import DEFAULT_CONFIG_DIRNAME
from .errors import AppError
from .projects import default_state_root
from .private_files import atomic_write_private, read_small_private_file


SCHEMA_VERSION = 1
FILENAME = "agent-access.json"
PATH_ENV = "KASSIBER_AGENT_ACCESS_FILE"
_MAX_BYTES = 1024

GRANT_READ = "read"
GRANT_OPERATOR = "operator"
GRANT_ACCOUNTING_DECISIONS = "accounting_decisions"
# Cumulative, lowest first; the operator broker's capability names.
GRANTS = (GRANT_READ, GRANT_OPERATOR, GRANT_ACCOUNTING_DECISIONS)
DEFAULT_GRANT = GRANT_READ
DEFAULT_AGENT_ACCESS = AI_AGENT_ACCESS_RESTRICTED
AGENT_ACCESS_MODES = AI_AGENT_ACCESS_MODES


@dataclass(frozen=True)
class _Preference:
    mcp_enabled: bool
    ai_features_enabled: bool | None
    grant: str
    agent_access: str
    # Whether the user recorded a grant/access choice (the desktop preselects
    # its recommendation only for a first enable).
    access_recorded: bool


_OFF = _Preference(False, None, DEFAULT_GRANT, DEFAULT_AGENT_ACCESS, False)


def preference_path() -> Path:
    override = os.environ.get(PATH_ENV)
    if override:
        return Path(override).expanduser()
    return default_state_root() / DEFAULT_CONFIG_DIRNAME / FILENAME


def _read(path: Path) -> _Preference:
    """Anything unreadable is off; unknown grant values fall back to read."""

    raw = read_small_private_file(path, _MAX_BYTES)
    if raw is None:
        return _OFF
    try:
        payload = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return _OFF
    if (
        not isinstance(payload, dict)
        or type(payload.get("schema_version")) is not int
        or payload["schema_version"] != SCHEMA_VERSION
    ):
        return _OFF
    mcp_enabled = payload.get("mcp_enabled")
    ai_features = payload.get("ai_features_enabled")
    grant = payload.get("grant")
    agent_access = payload.get("agent_access")
    return _Preference(
        mcp_enabled=mcp_enabled is True,
        ai_features_enabled=ai_features if isinstance(ai_features, bool) else None,
        grant=grant if grant in GRANTS else DEFAULT_GRANT,
        agent_access=(
            agent_access if agent_access in AGENT_ACCESS_MODES else DEFAULT_AGENT_ACCESS
        ),
        access_recorded="grant" in payload or "agent_access" in payload,
    )


def normalize_grant(value: Any) -> str:
    if value not in GRANTS:
        raise AppError(
            "the external agent grant must be read, operator or accounting_decisions",
            code="validation",
            hint="Admin work is never granted to agents.",
            retryable=False,
        )
    return value


def normalize_agent_access(value: Any) -> str:
    if value not in AGENT_ACCESS_MODES:
        raise AppError(
            "external agent access must be full or restricted",
            code="validation",
            retryable=False,
        )
    return value


def grant_allows(granted: str, required: str) -> bool:
    """Whether a cumulative grant covers ``required`` (both from GRANTS)."""

    return GRANTS.index(granted) >= GRANTS.index(required)


def narrower_grant(first: str, second: str) -> str:
    return first if GRANTS.index(first) <= GRANTS.index(second) else second


def agent_access_status(path: Path | None = None) -> dict[str, Any]:
    preference = _read(path or preference_path())
    reason = None
    if not preference.mcp_enabled:
        reason = "mcp_disabled"
    elif preference.ai_features_enabled is False:
        reason = "ai_features_disabled"
    return {
        "mcp_enabled": preference.mcp_enabled,
        "ai_features_enabled": preference.ai_features_enabled,
        "mcp_available": reason is None,
        "reason": reason,
        "grant": preference.grant,
        "agent_access": preference.agent_access,
        "access_recorded": preference.access_recorded,
    }


def set_agent_access(
    *,
    mcp_enabled: bool | None = None,
    ai_features_enabled: bool | None = None,
    grant: str | None = None,
    agent_access: str | None = None,
    path: Path | None = None,
) -> dict[str, Any]:
    """Update any field, keeping the others; the desktop and CLI share it."""

    if grant is not None:
        normalize_grant(grant)
    if agent_access is not None:
        normalize_agent_access(agent_access)
    destination = path or preference_path()
    current = _read(destination)
    document: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "mcp_enabled": current.mcp_enabled if mcp_enabled is None else bool(mcp_enabled),
        "ai_features_enabled": (
            current.ai_features_enabled
            if ai_features_enabled is None
            else bool(ai_features_enabled)
        ),
    }
    if grant is not None or agent_access is not None or current.access_recorded:
        # Record both once either is chosen, so a later read never mixes a
        # recorded grant with a defaulted access mode.
        document["grant"] = current.grant if grant is None else grant
        document["agent_access"] = (
            current.agent_access if agent_access is None else agent_access
        )
    atomic_write_private(destination, json.dumps(document, sort_keys=True) + "\n")
    return agent_access_status(destination)


def require_mcp_access(path: Path | None = None) -> dict[str, Any]:
    """Refuse unless agents are on; return the status with the user's grant."""

    status = agent_access_status(path)
    if status["mcp_available"]:
        return status
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


__all__ = [
    "AGENT_ACCESS_MODES",
    "AI_AGENT_ACCESS_FULL",
    "DEFAULT_AGENT_ACCESS",
    "DEFAULT_GRANT",
    "GRANTS",
    "agent_access_status",
    "grant_allows",
    "narrower_grant",
    "normalize_agent_access",
    "normalize_grant",
    "preference_path",
    "require_mcp_access",
    "set_agent_access",
]
