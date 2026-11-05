"""Write crates/ai/src/data/catalog.json: Python's AI tool catalog.

    uv run --locked python crates/ai/scripts/gen_catalog.py [--check]

The Rust crate embeds the ``tools`` list and the skill-reference index text.
``constants`` records the Python sets and limits the Rust code keeps as
reviewed source constants (so mutation entries can target them); the Rust
tests compare both against this file, and ``tests/test_rust_ai_tools.py``
compares this file against the live Python registries.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TARGET = Path(__file__).resolve().parents[1] / "src" / "data" / "catalog.json"
sys.path.insert(0, str(ROOT))


def document() -> dict:
    from kassiber import daemon
    from kassiber.ai import contracts, prompt, tools
    from kassiber.cli import chat as cli_chat
    from kassiber.core import chat_history, ui_snapshot

    entries = []
    for tool in tools.TOOL_CATALOG:
        entries.append(
            {
                "name": tool.name,
                "wire_name": tool.wire_name,
                "description": tool.description,
                # Literal key order: the provider sees the schema as written.
                "parameters": tool.parameters,
                "kind_class": tool.kind_class,
                "daemon_kind": tool.daemon_kind,
                "summary_template": tool.summary_template,
                "egresses": tool.egresses,
            }
        )
    aliases = {
        alias: entry.name
        for alias, entry in tools.TOOL_BY_NAME.items()
        if alias not in {entry.name, entry.provider_name}
    }
    return {
        "tools": entries,
        "aliases": dict(sorted(aliases.items())),
        "skill_reference_index": tools.SKILL_REFERENCE_INDEX,
        # Chat (R6-7): the product prompt and the page-metric definitions
        # the model reads, embedded as Python writes them.
        "system_prompt": prompt.DEFAULT_KASSIBER_SYSTEM_PROMPT,
        "screen_metric_definitions": {
            "grouped": ui_snapshot.screen_context_metric_definitions(True),
            "ungrouped": ui_snapshot.screen_context_metric_definitions(False),
        },
        "constants": {
            "ai_screen_routes": sorted(daemon._AI_SCREEN_ROUTES),
            "pending_ai_cancel_ttl_seconds": daemon.PENDING_AI_CANCEL_TTL_SECONDS,
            "max_pending_ai_cancels": daemon.MAX_PENDING_AI_CANCELS,
            "cut_off_finish_reasons": sorted(daemon._AI_CUT_OFF_FINISH_REASONS),
            "chat_history_modes": list(chat_history.AI_CHAT_HISTORY_MODES),
            "session_title_max_chars": chat_history.SESSION_TITLE_MAX_CHARS,
            "provider_status_notices": dict(contracts.PROVIDER_STATUS_NOTICES),
            "cli_finish_notices": dict(cli_chat._FINISH_NOTICES),
            "cli_repl_help": cli_chat._REPL_HELP,
            "copilot_incognito_notice": cli_chat.COPILOT_INCOGNITO_NOTICE,
            "sensitive_argument_key_parts": list(tools.SENSITIVE_ARGUMENT_KEY_PARTS),
            "skill_reference_names": list(tools.SKILL_REFERENCE_NAMES),
            "tool_profile_names": list(tools.TOOL_PROFILE_NAMES),
            "core_tool_names": sorted(tools.CORE_TOOL_NAMES),
            "review_tool_names": sorted(tools.REVIEW_TOOL_NAMES),
            "tool_capability_names": list(tools.TOOL_CAPABILITY_NAMES),
            "core_capability_tool_names": sorted(tools._CORE_TOOL_NAMES),
            "once_only_consent": sorted(daemon.AI_TOOL_ONCE_ONLY_CONSENT),
            "consent_timeout_seconds": daemon.AI_TOOL_CONSENT_TIMEOUT_SECONDS,
            "auto_journal_refresh_tools": sorted(daemon._AI_AUTO_JOURNAL_REFRESH_TOOL_NAMES),
            "local_custody_read_kinds": sorted(daemon._LOCAL_CUSTODY_READ_DAEMON_KINDS),
            "auto_context_max_chars": daemon.AUTO_CONTEXT_MAX_CHARS,
            "auto_context_entry_max_chars": daemon.AUTO_CONTEXT_ENTRY_MAX_CHARS,
            "auto_context_list_limit": daemon.AUTO_CONTEXT_LIST_LIMIT,
            "auto_context_string_limit": daemon.AUTO_CONTEXT_STRING_LIMIT,
            "swap_review_default_limit": daemon.SWAP_REVIEW_DEFAULT_LIMIT,
        },
    }


def render() -> str:
    return json.dumps(document(), indent=1, ensure_ascii=True) + "\n"


def main(argv: list[str]) -> int:
    text = render()
    if "--check" in argv:
        if TARGET.read_text(encoding="utf-8") != text:
            print(f"{TARGET} is stale; rerun {Path(__file__).name}", file=sys.stderr)
            return 1
        return 0
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
