"""Repro: a CLI provider's native session cursor is never resumed.

After a turn, ``_run_ai_chat_tool_loop`` (and the tool-free stream) records
the provider's cursor under ``_provider_session_history_fingerprint`` of the
turn's messages plus the answer. The next request carries the whole visible
transcript, which already ends with the new prompt, and the lookup
fingerprints that, so the two never match and every turn starts a fresh
native thread. In the tool loop the recorded answer is also only the last
leg's text, not what the user saw. The Rust port keeps this for parity
(tests/test_rust_ai_chat.py).

    uv run --locked python tests/rust_port/repro_chat_cursor_never_resumes.py
"""
from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber import daemon

    registry = daemon.ActiveAiChats()
    first = [{"role": "user", "content": "hello"}]
    answer = "all good"
    registry.remember_provider_session(
        chat_session_id="chat-1",
        provider_name="codex",
        provider_session_id="native-cursor-1",
        history_fingerprint=daemon._provider_session_history_fingerprint(
            [*first, {"role": "assistant", "content": answer}]
        ),
    )
    second = [*first, {"role": "assistant", "content": answer}, {"role": "user", "content": "thanks"}]
    resumed = registry.provider_session(
        chat_session_id="chat-1",
        provider_name="codex",
        history_fingerprint=daemon._provider_session_history_fingerprint(second),
    )
    print(f"cursor resumed for the next turn: {resumed!r}")
    return 0 if resumed is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
