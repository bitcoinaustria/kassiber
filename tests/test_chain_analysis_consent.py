"""Exercise acquisition through the real AI loop and once-only consent broker.

Only the provider response and node HTTP transport are replaced with local data.
Planning, consent previews, schema validation, scope guards, dispatch and storage
run unchanged. DNS and sockets are intercepted, including loopback attempts.
"""
from __future__ import annotations

import json
import queue
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.core import chain_analysis_acquisition as acquisition
from kassiber.db import open_db, set_setting
from tests.test_chain_analysis_acquisition import FakeHTTP, raw, tid
from tests.test_custody_component_surfaces import NOW, _fixture


@pytest.fixture
def book(tmp_path, monkeypatch):
    monkeypatch.delenv("KASSIBER_NO_EGRESS", raising=False)
    _fixture(tmp_path)
    conn = open_db(str(tmp_path))
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "profile")
    conn.execute("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES('node','esplora','bitcoin','main','http://127.0.0.1:18443/api',?,?)", (NOW, NOW))
    from kassiber.core.book_network import plan_book_network, apply_book_network
    scope = {"environment":"main", "declared_wallet_ids":[row[0] for row in conn.execute("SELECT id FROM wallets WHERE profile_id='profile'")]}
    preview = plan_book_network(conn, "profile", scope)
    apply_book_network(conn, "profile", {**scope, "plan_id":preview["plan_id"]})
    conn.commit()
    runtime = daemon.AiToolRuntime(str(tmp_path), {}, queue.Queue(), {
        "scope_workspace_id": "ws", "scope_profile_id": "profile",
        "provider_kind": "local", "provider_on_device": True,
    })
    with patch.object(daemon, "_run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)):
        yield conn, runtime
    conn.close()


class ConsentOutput:
    def __init__(self, on_consent):
        self.events = []
        self.on_consent = on_consent

    def write(self, payload):
        self.events.append(payload)
        if payload["kind"] == "ai.chat.tool_consent_required":
            self.on_consent(payload["data"])


def run_acquisition_chat(book, decision, *, after_preview=None, rounds=1, on_device=True):
    conn, runtime = book
    runtime.maintenance_state["provider_on_device"] = on_device
    chats = daemon.ActiveAiChats()
    _, active = chats.register("acquisition-loop")
    # An unsolicited or stale UI approval cannot authorize a future call.
    assert not active.consent.record("acquire-0", "allow_once")
    http = FakeHTTP({
        "/block-height/0": acquisition.GENESIS[("bitcoin", "main")].encode(),
        f"/tx/{tid(2)}": raw(2),
    })
    validated = daemon._ai_chat_args({
        "model": "synthetic-local-model", "tools_enabled": True,
        "messages": [{"role": "user", "content": "Use chain analysis to plan acquisition from my configured node, then request permission to fetch missing history."}],
    })
    prompts = []

    def on_consent(data):
        # Each node read must follow its own observed user approval. Even an
        # allow_session response cannot silently grant the second acquisition.
        assert len(http.calls) == 2 * len(prompts)
        assert data["arguments_preview"]["status"] == "ready"
        assert data["arguments_preview"]["plan"]["effects"]["egresses"] is True
        prompts.append(data)
        if after_preview:
            after_preview(conn)
        if decision == "cancelled":
            active.cancel_event.set()
            active.consent.notify_cancelled()
        elif decision != "consent_timeout":
            assert active.consent.record(data["call_id"], decision)

    output = ConsentOutput(on_consent)
    turn_number = 0

    def provider_turn(_rid, _client, _validated, context, offered, _out, _cancel):
        nonlocal turn_number
        if turn_number >= rounds:
            return daemon.AiToolTurnResult([], "Investigation complete.", "", "stop", [])
        entry = daemon.get_tool("ui.chain_analysis.acquire.apply")
        assert (entry.provider_name in {tool["name"] for tool in offered}) is on_device
        plan = acquisition.plan_acquisition(conn, "profile", {
            "backend": "node", "subject": tid(2), "chain": "bitcoin",
            "network": "main", "direction": "backward",
        })
        call = {"id": f"acquire-{turn_number}", "function": {"name": entry.provider_name, "arguments": json.dumps({"plan": plan})}}
        turn_number += 1
        return daemon.AiToolTurnResult([call], "", "", "tool_calls", [])

    with (
        patch.object(daemon, "_stream_ai_chat_tool_turn", provider_turn),
        patch.object(daemon, "AI_TOOL_CONSENT_TIMEOUT_SECONDS", 0),
        patch.object(daemon, "_write_ai_chat_terminal") as terminal,
        patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http),
        patch("socket.getaddrinfo", side_effect=AssertionError("unexpected DNS")) as dns,
        patch("socket.socket.connect", side_effect=AssertionError("unexpected socket")) as connect,
    ):
        daemon._run_ai_chat_tool_loop(
            "acquisition-loop", SimpleNamespace(last_provider_session_id=None),
            {"name": "synthetic", "kind": "local", "base_url": "http://localhost"},
            validated, output, active, runtime, chats,
        )
    dns.assert_not_called()
    connect.assert_not_called()
    terminal.assert_called_once()
    return http, prompts, output.events, terminal.call_args


@pytest.mark.parametrize("decision", ["allow_once", "deny", "cancelled", "consent_timeout"])
def test_actual_ai_loop_acquires_only_after_user_approval(book, decision):
    conn, _ = book
    http, prompts, events, terminal = run_acquisition_chat(book, decision)
    assert len(prompts) == 1
    assert len(http.calls) == (2 if decision == "allow_once" else 0)
    assert conn.execute("SELECT COUNT(*) FROM chain_analysis_observations").fetchone()[0] == (decision == "allow_once")
    results = [event["data"] for event in events if event["kind"] == "ai.chat.tool_result" and event["data"]["call_id"] == "acquire-0"]
    if decision == "cancelled":
        assert not results
        assert terminal.args[4] == "cancelled"
    elif decision == "allow_once":
        assert results[-1]["ok"] is True
        assert results[-1]["envelope"]["data"]["acquired_count"] == 1
    else:
        assert results[-1]["reason"] == ("user_denied" if decision == "deny" else "consent_timeout")


@pytest.mark.parametrize("changed", ["backend", "book", "kill_switch"])
def test_approval_cannot_override_a_changed_plan_scope_or_disabled_network(book, monkeypatch, changed):
    conn, _ = book

    def change_state(conn):
        if changed == "backend":
            # The URL identity changes while updated_at deliberately stays fixed.
            conn.execute("UPDATE backends SET url='https://different.example/api'")
        elif changed == "book":
            set_setting(conn, "context_profile", "another-book")
        else:
            monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")

    http, prompts, events, _ = run_acquisition_chat(book, "allow_once", after_preview=change_state)
    assert len(prompts) == 1
    assert not http.calls
    results = [event["data"] for event in events if event["kind"] == "ai.chat.tool_result" and event["data"]["call_id"] == "acquire-0"]
    assert results[-1]["reason"] == {"backend": "chain_analysis_stale", "book": "stale_context", "kill_switch": "network_egress_disabled"}[changed]
    assert conn.execute("SELECT COUNT(*) FROM chain_analysis_observations").fetchone()[0] == 0


def test_acquisition_is_once_only_even_when_ui_asks_to_allow_the_session(book):
    http, prompts, events, _ = run_acquisition_chat(book, "allow_session", rounds=2)
    assert len(prompts) == 2
    assert len(http.calls) == 4
    assert all(event["data"]["ok"] for event in events if event["kind"] == "ai.chat.tool_result" and event["data"]["call_id"].startswith("acquire-"))


def test_off_device_provider_cannot_call_unadvertised_acquisition_tool(book):
    http, prompts, events, _ = run_acquisition_chat(book, "allow_once", on_device=False)
    assert not prompts
    assert not http.calls
    result = next(event["data"] for event in events if event["kind"] == "ai.chat.tool_result" and event["data"]["call_id"] == "acquire-0")
    assert result["reason"] == "tool_not_advertised"
