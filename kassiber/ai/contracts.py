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
}
CLI_PROVIDER_LOCATORS = tuple(CLI_PROVIDER_BY_LOCATOR)


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


@dataclass
class ResponsesRequestContext:
    """Explicit stateless Responses input owned by one chat/tool loop."""

    instructions: str | None
    input_items: list[dict[str, Any]]
