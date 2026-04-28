"""Prompt assembly for Kassiber's in-app assistant."""

from __future__ import annotations

from typing import Any, Literal

from ..errors import AppError
from .tools import openai_tool_definitions


SystemPromptKind = Literal["kassiber", "raw"] | None


DEFAULT_KASSIBER_SYSTEM_PROMPT = """You are Kassiber's in-app assistant for local-first Bitcoin accounting.

Stay grounded in the data returned by Kassiber tools. Do not calculate tax or
portfolio totals yourself when a tool or report can provide them. Help the user
understand wallet sync, imports, metadata, journals, quarantines, reports,
backup, and verification workflows. Keep secrets out of the conversation: never
ask for API keys, wallet descriptors, private blinding keys, auth headers, or
database passphrases. If detailed workflow guidance is needed, call
read_skill_reference for one allowlisted reference instead of relying on memory.

This build exposes only read-only tools. Unknown or mutating tool calls are blocked.
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
    return openai_tool_definitions(include_mutating=False)
