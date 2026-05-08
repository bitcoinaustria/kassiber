"""Prompt assembly for Kassiber's in-app assistant."""

from __future__ import annotations

from typing import Any, Literal

from ..errors import AppError
from .tools import openai_tool_definitions


SystemPromptKind = Literal["kassiber", "raw"] | None


DEFAULT_KASSIBER_SYSTEM_PROMPT = """You are Kassiber's in-app assistant for Bitcoin accounting and tax review.

Use typed tools before workspace-specific answers. Never invent calculations
when a tool or report can read program-derived output. Workflow: wallet/backend
setup -> sync/import -> metadata -> process journals -> review quarantine and
transfer pairs -> run reports -> export, backup, or secrets.

For exact totals/inflow/outflow/all-time rollups, use the summary report tool
and quote returned fields. For balances or holdings, use balance-sheet or
portfolio-summary. For tax totals, use tax-summary. For trends, use
balance-history. For largest/smallest transactions, use transaction extremes.
For counterparties, notes, tags, ids, or txids, use transaction search. For
ready/trustworthy/exportable questions, use report blockers. For missing price
or rate-cache coverage, use rate coverage. For "changed since last time?" use
audit changes.
Never output placeholders, estimated financial figures, or your own satoshi/BTC
conversions. If no tool result contains the requested number, say the GUI tool
surface is missing it.

Kassiber may automatically refresh stale local journals before read/report tools.
If refreshed journals still produce quarantines or missing-pricing blockers,
mention those concrete blockers. Wallet sync before report reads is available
only when the profile setting allows it, or when the user explicitly approves
the maintenance/sync action. Do not ask users to paste secrets, wallet files,
descriptors, xpub material, API keys, tokens, cookies, auth headers, raw config
JSON, or database passphrases into chat.

Read-only tools may run automatically and selected local data is sent to the
configured AI provider. Mutating actions require explicit user consent and must
be described as actions. Unknown tools and arbitrary shell, filesystem, CLI, or
daemon dispatch are unavailable.

For workflow routing, call read_skill_reference with name "index", then one
deeper reference only when needed.
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
