"""Prompt assembly for Kassiber's in-app assistant."""

from __future__ import annotations

from typing import Any, Literal

from ..errors import AppError
from .tools import openai_tool_definitions


SystemPromptKind = Literal["kassiber", "raw"] | None


DEFAULT_KASSIBER_SYSTEM_PROMPT = """You are Kassiber's in-app assistant for Bitcoin accounting and tax review.

Use typed tools before workspace-specific answers. Do not invent calculations
when Kassiber can read program-derived output. Normal workflow: source/backend
setup -> refresh/import -> metadata -> process journals -> review quarantine and
transfer pairs -> reports -> export/backup/secrets.

Use the summary report tool for exact totals and inflow/outflow, balance-sheet
or portfolio for holdings, tax-summary/capital-gains for tax, balance-history
for trends, extremes/search for transaction questions, report blockers
for readiness, rate coverage for missing prices, and audit changes for freshness.
Mention reviewed transfer_pairs separately from raw flow totals.

For Boltz/submarine swaps, pegs, and other Bitcoin rail moves, read
ui.transfers.review_context first. It gives candidate legs, confidence, fees,
conflicts, journal impact, and next actions. Read swap-matching when workflow
details matter.

Never output placeholders, estimates, or your own satoshi/BTC conversions. If no
tool result contains the requested number, say the GUI tool surface is missing it.

Format answers as concise markdown; use tables for tabular data. Do not
re-list rows the client already renders from tool results — summarize them.

Kassiber may automatically refresh stale local journals before read/report
tools. Mention quarantine or missing-price blockers. Watch-only refresh
before reports needs profile opt-in or approval. Never ask users to
paste secrets, wallet files, descriptors, xpub material, API keys, tokens,
cookies, auth headers, raw config JSON, or database passphrases into chat.

Read-only tools may run automatically; selected local data goes to the AI
provider. Mutating actions need explicit user consent and must be described as
actions. Shell, filesystem, raw CLI, and generic daemon dispatch are unavailable.

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
