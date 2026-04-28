"""Prompt assembly for Kassiber's in-app assistant."""

from __future__ import annotations

from typing import Any, Literal

from ..errors import AppError
from .tools import openai_tool_definitions


SystemPromptKind = Literal["kassiber", "raw"] | None


DEFAULT_KASSIBER_SYSTEM_PROMPT = """You are Kassiber's in-app assistant for local-first Bitcoin accounting and tax review.

Use Kassiber's typed tools before answering workspace-specific questions. Never
invent calculations when a tool or report can read program-derived output. The
usual workflow is: wallet/backend setup -> sync or import -> metadata tags,
notes, and exclusions -> process journals -> review quarantine and transfer
pairs -> run reports -> export, back up, or handle secrets.

Tell users that journals must be reprocessed after transaction imports, wallet
syncs, transfer pairing, metadata changes, exclusions, rate syncs, or rate
overrides before reports are trusted. Do not ask users to paste secrets, wallet
files, descriptors, xpub material, API keys, tokens, cookies, auth headers, raw
config JSON, or database passphrases into chat.

Read-only tools may run automatically and their selected local data is sent to
the configured AI provider. Mutating actions require explicit user consent and
must be described as actions, not just information. Unknown tools and arbitrary
shell, filesystem, CLI, or daemon dispatch are unavailable.

For workflow routing, call read_skill_reference with name "index" first, then
read one deeper allowlisted reference only when needed.
"""


def normalize_system_prompt_kind(raw: object, *, tools_enabled: bool) -> SystemPromptKind:
    if raw is None:
        return "kassiber" if tools_enabled else None
    if raw in ("kassiber", "raw"):
        return raw  # type: ignore[return-value]
    raise AppError(
        "ai.chat system_prompt_kind must be 'kassiber', 'raw', or null",
        code="validation",
        details={"system_prompt_kind": raw},
        retryable=False,
    )


def build_chat_messages(
    messages: list[dict[str, Any]],
    *,
    system_prompt_kind: SystemPromptKind,
    system_prompt: str | None = None,
) -> list[dict[str, Any]]:
    if system_prompt is not None and system_prompt_kind != "raw":
        raise AppError(
            "ai.chat system_prompt is only accepted when system_prompt_kind is raw",
            code="validation",
            retryable=False,
        )
    if system_prompt_kind == "raw":
        if not isinstance(system_prompt, str):
            raise AppError(
                "ai.chat raw system_prompt must be a string",
                code="validation",
                retryable=False,
            )
        return [{"role": "system", "content": system_prompt}, *messages]
    if system_prompt_kind == "kassiber":
        return [{"role": "system", "content": DEFAULT_KASSIBER_SYSTEM_PROMPT}, *messages]
    return list(messages)


def build_openai_tools() -> list[dict[str, Any]]:
    return openai_tool_definitions(include_mutating=True)
