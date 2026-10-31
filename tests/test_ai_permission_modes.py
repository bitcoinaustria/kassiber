"""Chat permission modes.

Approval asks before every change; Auto approves a change another tool can
undo and that stays on this machine; Full access approves everything. The
daemon decides, and a mode switch mid-turn releases what the new mode covers.
"""

from __future__ import annotations

import io
import json
import queue
import sqlite3
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import custody_ai_audit
from kassiber.db import open_db
from kassiber.errors import AppError


def _chat_args(**extra):
    return daemon._ai_chat_args(
        {"model": "m", "messages": [{"role": "user", "content": "hi"}], **extra}
    )


class PermissionModeArgsTest(unittest.TestCase):
    def test_absent_mode_is_approval_and_every_tool_is_offered(self):
        validated = _chat_args()
        self.assertEqual(validated["permission_mode"], "approval")
        self.assertEqual(validated["tool_profile"], "full")

    def test_modes_are_validated(self):
        for mode in ("approval", "auto", "full_access"):
            self.assertEqual(_chat_args(permission_mode=mode)["permission_mode"], mode)
        with self.assertRaises(AppError) as raised:
            _chat_args(permission_mode="yolo")
        self.assertEqual(raised.exception.code, "validation")


class ConsentStateTest(unittest.TestCase):
    def test_preapproval_follows_mode_reversibility_and_egress(self):
        state = daemon.AiToolConsentState()
        local = {"reversible": True, "egresses": False}
        self.assertIsNone(state.preapproval("ui.transfers.pair", **local))
        state.set_permission_mode("auto")
        self.assertEqual(state.preapproval("ui.transfers.pair", **local), "mode_auto")
        self.assertIsNone(
            state.preapproval("ui.wallets.sync", reversible=False, egresses=True)
        )
        self.assertIsNone(
            state.preapproval("ui.reports.export", reversible=False, egresses=False)
        )
        state.set_permission_mode("full_access")
        self.assertEqual(
            state.preapproval("ui.review.apply", reversible=False, egresses=False),
            "mode_full_access",
        )

    def test_a_mode_switch_releases_a_waiting_call_it_covers(self):
        state = daemon.AiToolConsentState("auto")
        cancel = threading.Event()
        decisions: list[str] = []
        waiter = threading.Thread(
            target=lambda: decisions.append(
                state.wait(
                    call_id="sync",
                    tool_name="ui.wallets.sync",
                    cancel_event=cancel,
                    timeout=10,
                    reversible=False,
                    egresses=True,
                )
            )
        )
        waiter.start()
        deadline = time.monotonic() + 5
        while "sync" not in state._pending_risk and time.monotonic() < deadline:
            time.sleep(0.01)
        # Auto never approves network access; Full access does.
        self.assertEqual(state.set_permission_mode("auto"), [])
        self.assertEqual(state.set_permission_mode("full_access"), ["sync"])
        waiter.join(5)
        self.assertEqual(decisions, ["mode_full_access"])


class ChatLoopPermissionModeTest(unittest.TestCase):
    def _validated(self, mode):
        return {
            "messages": [{"role": "user", "content": "fix it"}],
            "model": "m",
            "options": {},
            "session_id": None,
            "persist": False,
            "system_prompt_kind": "kassiber",
            "system_prompt": None,
            "tools_enabled": True,
            "tool_profile": "full",
            "permission_mode": mode,
            "tool_loop_max_iterations": 2,
            "screen_context": None,
            "attachments": [],
        }

    def _start(self, mode, tool_name, arguments):
        calls = [{"id": "call_1", "function": {"name": tool_name, "arguments": json.dumps(arguments)}}]
        rounds = {"n": 0}

        def fake_turn(*_args, **_kwargs):
            rounds["n"] += 1
            first = rounds["n"] == 1
            return daemon.AiToolTurnResult(
                tool_calls=calls if first else [],
                content="" if first else "done",
                reasoning="",
                finish_reason="tool_calls" if first else "stop",
                response_output=[],
                notices=(),
            )

        executed: list[dict] = []

        def fake_execute(call, runtime, *, custody_audit=None):
            executed.append({"name": call.name, "decision": custody_audit.consent_decision})
            return {"ok": True, "envelope": {"kind": call.name, "data": {}}}

        stream = io.StringIO()
        chats = daemon.ActiveAiChats()
        _key, chat = chats.register("req-1")
        chat.consent.set_permission_mode(mode)
        runtime = daemon.AiToolRuntime("/not-used", {}, queue.Queue(), {"egress_after_id": 0})
        patches = [
            patch.object(daemon, "_stream_ai_chat_tool_turn", fake_turn),
            patch.object(daemon, "_run_auto_read_tools", lambda **_kwargs: None),
            patch.object(daemon, "_execute_mutating_ai_tool", fake_execute),
            patch.object(daemon, "_ai_plan_consent_preview", lambda _r, _e, args: (args, None)),
            patch.object(daemon, "_ai_review_consent_preview", lambda *_a: {"status": "unavailable"}),
        ]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)

        def run():
            daemon._run_ai_chat_tool_loop(
                "req-1",
                SimpleNamespace(last_provider_session_id=None),
                {"name": "p", "kind": "remote", "base_url": "https://example.test/v1"},
                self._validated(mode),
                daemon._OutputChannel(stream),
                chat,
                runtime,
                chats,
            )

        thread = threading.Thread(target=run)
        thread.start()
        return thread, chats, chat, stream, executed

    @staticmethod
    def _records(stream):
        return [json.loads(line) for line in stream.getvalue().splitlines()]

    def _wait_pending(self, chat):
        deadline = time.monotonic() + 5
        while not chat.consent._pending_risk and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(chat.consent._pending_risk, "the call never waited for consent")

    def test_auto_runs_a_reversible_local_change_without_asking(self):
        thread, _chats, _chat, stream, executed = self._start(
            "auto", "ui_transfers_pair", {"tx_out": "o", "tx_in": "i"}
        )
        thread.join(10)
        records = self._records(stream)
        kinds = [record["kind"] for record in records]
        self.assertNotIn("ai.chat.tool_consent_required", kinds)
        first = next(r for r in records if r["kind"] == "ai.chat.tool_call")
        self.assertFalse(first["data"]["needs_consent"])
        self.assertEqual(first["data"]["approval"], "auto")
        self.assertEqual(executed, [{"name": "ui_transfers_pair", "decision": "mode_auto"}])

    def test_auto_asks_for_network_access_and_a_switch_to_full_access_releases_it(self):
        thread, chats, chat, stream, executed = self._start("auto", "ui_wallets_sync", {})
        self._wait_pending(chat)
        consent = next(
            r for r in self._records(stream) if r["kind"] == "ai.chat.tool_consent_required"
        )
        self.assertTrue(consent["data"]["egresses"])
        self.assertFalse(consent["data"]["reversible"])
        self.assertTrue(consent["data"]["can_allow_session"])
        self.assertEqual(chats.set_permission_mode("req-1", "full_access"), ["call_1"])
        thread.join(10)
        tool_calls = [r for r in self._records(stream) if r["kind"] == "ai.chat.tool_call"]
        self.assertEqual(tool_calls[0]["data"]["approval"], "required")
        self.assertEqual(tool_calls[-1]["data"]["approval"], "full_access")
        self.assertEqual(executed, [{"name": "ui_wallets_sync", "decision": "mode_full_access"}])

    def test_full_access_runs_a_once_only_tool_without_asking(self):
        thread, _chats, _chat, stream, executed = self._start(
            "full_access",
            "ui_transfers_components_apply",
            {"action": "supersede", "component_id": "c", "expected_input_version": 1},
        )
        thread.join(10)
        kinds = [record["kind"] for record in self._records(stream)]
        self.assertNotIn("ai.chat.tool_consent_required", kinds)
        self.assertEqual(executed[0]["decision"], "mode_full_access")

    def test_approval_asks_and_a_click_reads_as_required(self):
        thread, chats, chat, stream, executed = self._start(
            "approval", "ui_transfers_pair", {"tx_out": "o", "tx_in": "i"}
        )
        self._wait_pending(chat)
        self.assertTrue(chats.record_consent("req-1", "call_1", "allow_once"))
        thread.join(10)
        tool_calls = [r for r in self._records(stream) if r["kind"] == "ai.chat.tool_call"]
        self.assertEqual(tool_calls[0]["data"]["approval"], "required")
        self.assertTrue(tool_calls[0]["data"]["needs_consent"])
        self.assertEqual(tool_calls[-1]["data"]["approval"], "required")
        self.assertEqual(executed[0]["decision"], "allow_once")


class PermissionModeKindTest(unittest.TestCase):
    def test_kind_updates_a_running_chat_and_reports_an_unknown_one(self):
        ctx = SimpleNamespace(active_ai_chats=daemon.ActiveAiChats())
        _key, chat = ctx.active_ai_chats.register("req-9")
        response = daemon._handle_ai_chat_permission_mode(
            ctx, "r1", {"target_request_id": "req-9", "permission_mode": "auto"}
        )
        self.assertEqual(response["kind"], "ai.chat.permission_mode")
        self.assertEqual(
            response["data"],
            {"permission_mode": "auto", "released_call_ids": [], "updated": True},
        )
        self.assertEqual(chat.consent.permission_mode, "auto")
        missing = daemon._handle_ai_chat_permission_mode(
            ctx, "r2", {"target_request_id": "gone", "permission_mode": "auto"}
        )
        self.assertEqual(missing["data"]["reason"], "not_found")
        self.assertFalse(missing["data"]["updated"])
        with self.assertRaises(AppError):
            daemon._handle_ai_chat_permission_mode(
                ctx, "r3", {"target_request_id": "req-9", "permission_mode": "never"}
            )
        self.assertIn("ai.chat.permission_mode", daemon.SUPPORTED_KINDS)


class AuditModeDecisionTest(unittest.TestCase):
    LEGACY_TABLE = """
        CREATE TABLE custody_ai_assistance_audits (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            tool_name TEXT NOT NULL,
            daemon_kind TEXT NOT NULL,
            call_id TEXT NOT NULL,
            provider_kind TEXT NOT NULL,
            model TEXT NOT NULL,
            gap_id TEXT,
            candidate_fingerprint TEXT,
            facts_sha256 TEXT NOT NULL CHECK(length(facts_sha256) = 64),
            model_proposal_json TEXT NOT NULL DEFAULT '{}',
            final_proposal_json TEXT NOT NULL DEFAULT '{}',
            user_edited INTEGER NOT NULL DEFAULT 0 CHECK(user_edited IN (0, 1)),
            consent_decision TEXT NOT NULL CHECK(consent_decision IN (
                'allow_once', 'allow_session', 'deny', 'consent_timeout', 'cancelled'
            )),
            consent_requested_at TEXT NOT NULL,
            consent_decided_at TEXT NOT NULL,
            execution_status TEXT NOT NULL CHECK(execution_status IN (
                'executed', 'failed', 'denied', 'cancelled'
            )),
            execution_code TEXT,
            result_sha256 TEXT CHECK(result_sha256 IS NULL OR length(result_sha256) = 64),
            review_id TEXT,
            component_id TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TRIGGER trg_custody_ai_assistance_immutable
        BEFORE UPDATE ON custody_ai_assistance_audits
        BEGIN
            SELECT RAISE(ABORT, 'custody_ai_assistance_immutable');
        END;
    """

    def _record(self, conn, decision, call_id):
        return custody_ai_audit.append_assistance_record(
            conn,
            workspace_id="ws",
            profile_id="profile",
            tool_name="ui.custody.review.apply",
            daemon_kind="ui.custody.review.apply",
            call_id=call_id,
            provider_kind="remote",
            model="m",
            model_proposal={"gap_id": "gap"},
            final_proposal={"gap_id": "gap"},
            consent_decision=decision,
            consent_requested_at="2026-10-28T10:00:00Z",
            consent_decided_at="2026-10-28T10:00:00Z",
            execution_status="executed",
            result={"gap_id": "gap"},
        )

    def test_old_books_learn_mode_decisions_and_keep_their_rows(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-audit-modes-") as tmp:
            conn = open_db(Path(tmp))
            conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Book', 'now')")
            conn.execute(
                "INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country, "
                "tax_long_term_days, gains_algorithm, created_at) "
                "VALUES('profile', 'ws', 'Owner', 'EUR', 'generic', 365, 'FIFO', 'now')"
            )
            conn.execute("DROP TRIGGER trg_custody_ai_assistance_immutable")
            conn.execute("DROP TABLE custody_ai_assistance_audits")
            conn.executescript(self.LEGACY_TABLE)
            self._record(conn, "allow_once", "old")
            conn.commit()
            with self.assertRaises(sqlite3.IntegrityError):
                # The legacy CHECK knows only user decisions.
                self._record(conn, "mode_auto", "too-early")
            conn.rollback()
            conn.close()

            conn = open_db(Path(tmp))
            try:
                self.assertEqual(
                    [
                        tuple(row)
                        for row in conn.execute(
                            "SELECT call_id, consent_decision FROM custody_ai_assistance_audits"
                        )
                    ],
                    [("old", "allow_once")],
                )
                self._record(conn, "mode_auto", "auto")
                self._record(conn, "mode_full_access", "full")
                conn.commit()
                # Still append-only.
                with self.assertRaises(sqlite3.IntegrityError):
                    conn.execute(
                        "UPDATE custody_ai_assistance_audits SET model = 'x' WHERE call_id = 'old'"
                    )
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()


class SessionAllowTest(unittest.TestCase):
    """"Allow for this chat" outlives the answer it was given in."""

    def test_allow_session_carries_to_the_next_turn_of_the_same_chat(self):
        chats = daemon.ActiveAiChats()
        _, first = chats.register("turn-1")
        first.consent.session_allows.add("ui.transfers.pair")
        # A new chat learns its id from the terminal record of its first turn.
        chats.remember_session_allows("chat-a", first.consent.session_allows)

        _, second = chats.register("turn-2", session_id="chat-a")
        self.assertTrue(second.consent.has_session_allow("ui.transfers.pair"))
        _, other = chats.register("turn-3", session_id="chat-b")
        self.assertFalse(other.consent.has_session_allow("ui.transfers.pair"))

    def test_once_only_tools_never_take_a_chat_wide_allow(self):
        chats = daemon.ActiveAiChats()
        _, chat = chats.register("turn-1", session_id="chat-a")
        chat.consent.session_allows.add("ui.review.apply")
        self.assertFalse(chat.consent.has_session_allow("ui.review.apply"))

    def test_remembered_sessions_are_bounded(self):
        chats = daemon.ActiveAiChats()
        for index in range(daemon.AI_SESSION_ALLOWS_MAX + 10):
            chats.remember_session_allows(f"chat-{index}", {"ui.transfers.pair"})
        _, oldest = chats.register("turn", session_id="chat-0")
        self.assertFalse(oldest.consent.has_session_allow("ui.transfers.pair"))
