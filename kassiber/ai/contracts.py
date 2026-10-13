"""Shared contracts for Kassiber AI provider clients."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


DEFAULT_TIMEOUT_SECONDS = 120
CLI_DEFAULT_MODEL = "default"
CLI_PROVIDER_BY_LOCATOR = {
    "claude-cli://default": "claude",
    "codex-cli://default": "codex",
    "opencode-cli://default": "opencode",
    # Agent Client Protocol agents, run through the broker's generic ACP adapter.
    "copilot-cli://default": "copilot",
}
CLI_PROVIDER_LOCATORS = tuple(CLI_PROVIDER_BY_LOCATOR)

# Reasoning effort on the OpenAI-compatible HTTP wire. Kassiber's higher levels
# (xhigh, max, ultra) exist for the CLI agents; HTTP endpoints broadly accept
# only these three, so the higher levels are sent as "high", and "auto" (or a
# level Kassiber does not know) sends no effort at all.
HTTP_REASONING_EFFORTS = ("low", "medium", "high")
_HTTP_REASONING_EFFORT_CLAMP = {"xhigh": "high", "max": "high", "ultra": "high"}


def http_reasoning_effort(value: object) -> str | None:
    """The effort an HTTP request carries for a Kassiber level, or None."""

    if not isinstance(value, str):
        return None
    effort = value.strip().lower()
    if effort in HTTP_REASONING_EFFORTS:
        return effort
    return _HTTP_REASONING_EFFORT_CLAMP.get(effort)


def is_cli_provider_locator(value: object) -> bool:
    if not isinstance(value, str):
        return False
    return value.strip().lower() in CLI_PROVIDER_LOCATORS


def cli_provider_for_locator(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    return CLI_PROVIDER_BY_LOCATOR.get(value.strip().lower())


@dataclass(frozen=True)
class ChatDelta:
    """One provider-neutral chunk emitted by an AI client.

    ``delta`` keeps the daemon shape (``content``, ``reasoning``, and
    normalized ``tool_calls``). Responses clients also attach the terminal
    typed ``response_output`` so the daemon can replay protocol Items without
    flattening them back into chat messages.
    """

    delta: dict[str, Any]
    finish_reason: str | None
    raw: dict[str, Any]
    response_output: list[dict[str, Any]] | None = None
    # A provider notice for the turn (one of ``PROVIDER_STATUS_NOTICES``), with
    # an empty ``delta``. The daemon forwards it as an ``ai.chat.status``.
    status_phase: str | None = None


# Provider notices forwarded to callers, with their English label: a CLI agent
# declined fast mode (reported by the broker), or an HTTP endpoint refused the
# reasoning effort and the turn was retried without it. Only these phases
# cross: other broker statuses ("connecting") are progress chatter, and a
# provider-chosen phase must never become arbitrary UI copy.
PROVIDER_STATUS_NOTICES = {
    "fast_mode_unavailable": "Fast mode unavailable; answering at standard speed",
    "reasoning_effort_unavailable": "Reasoning effort unavailable; answering without it",
}


@dataclass
class ResponsesRequestContext:
    """Explicit stateless Responses input owned by one chat/tool loop."""

    instructions: str | None
    input_items: list[dict[str, Any]]
