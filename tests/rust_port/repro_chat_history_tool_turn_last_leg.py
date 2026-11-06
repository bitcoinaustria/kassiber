"""Repro: a tool turn's saved answer is only its last leg's text.

    uv run --locked python -m tests.rust_port.repro_chat_history_tool_turn_last_leg

`_run_ai_chat_tool_loop` streams every leg's text to the user as
`ai.chat.delta` records, so the desktop shows (and sends back as the next
request's assistant message) "Let me check. " followed by the final answer.
Round r7-6 binds the provider cursor to that whole visible answer
(`shown_parts`), but the terminal record and the persisted chat history still
take `content`, which each leg overwrites: the saved assistant message is only
the last leg's text. Reopening the chat from history therefore shows a
different answer than the user saw, and its transcript no longer matches the
cursor's branch, so a resumed history session starts a fresh native thread.

The Rust port mirrors this (crates/ai/src/chat/turn.rs passes the last leg's
`content` to `terminal`, and tests/test_rust_ai_chat.py compares the history
tables byte for byte with Python). Exit 0 means the bug reproduced.
"""
from __future__ import annotations

import io
import json
import queue
from types import SimpleNamespace
from unittest.mock import patch

from kassiber import daemon


def main() -> int:
    legs = [
        daemon.AiToolTurnResult(
            tool_calls=[{"id": "call-1", "type": "function",
                         "function": {"name": "unknown_tool", "arguments": "{}"}}],
            content="Let me check. ", reasoning="", finish_reason="tool_calls", response_output=[],
        ),
        daemon.AiToolTurnResult(
            tool_calls=[], content="Your balance is in the report.", reasoning="",
            finish_reason="stop", response_output=[],
        ),
    ]
    saved = []
    out = io.StringIO()
    validated = {
        "messages": [{"role": "user", "content": "What is my balance?"}],
        "model": "default", "options": {}, "session_id": "chat-1",
        "system_prompt_kind": "kassiber", "system_prompt": None, "tools_enabled": True,
        "tool_profile": "scoped", "tool_loop_max_iterations": 3, "screen_context": None,
        "attachments": [],
    }
    runtime = daemon.AiToolRuntime(data_root="/nonexistent/kassiber-repro", runtime_config={},
                                   main_thread_tasks=queue.Queue(), maintenance_state={})
    chats = daemon.ActiveAiChats()
    _key, chat = chats.register("req-1")

    def fake_leg(_request_id, _client, _validated, _context, _tools, channel, _cancel):
        leg = legs.pop(0)
        channel.write(daemon.build_envelope("ai.chat.delta", {"delta": {"content": leg.content}}))
        return leg

    def terminal(*_args, assistant_content, **_kwargs):
        saved.append(assistant_content)

    with patch.object(daemon, "_stream_ai_chat_tool_turn", fake_leg), \
            patch.object(daemon, "_run_auto_read_tools", lambda **_kwargs: None), \
            patch.object(daemon, "_write_ai_chat_terminal", terminal):
        daemon._run_ai_chat_tool_loop(
            "req-1", SimpleNamespace(last_provider_session_id="cursor-1"),
            {"name": "codex", "base_url": "codex-cli://default"}, validated,
            daemon._OutputChannel(out), chat, runtime, chats,
        )
    shown = "".join(
        record["data"]["delta"]["content"]
        for record in map(json.loads, out.getvalue().splitlines())
        if record.get("kind") == "ai.chat.delta"
    )
    print(f"shown to the user: {shown!r}")
    print(f"saved as the answer: {saved!r}")
    return 0 if saved == ["Your balance is in the report."] and shown != saved[0] else 1


if __name__ == "__main__":
    raise SystemExit(main())
