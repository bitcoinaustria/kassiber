"""Agent sessions: the desktop's "Unlock for agents" lease in the broker.

An agent lease admits only `mcp call`, only from processes its grantor
allowed with the control secret, and ends after an idle timeout.
"""

from __future__ import annotations

import os
import socket
import sys
import tempfile
import time
import unittest
from unittest import mock

from kassiber.command_capabilities import Capability
from kassiber.errors import AppError
from kassiber.operator.service import OperationResult, OperatorService

CONTROL = bytearray(b"c" * 32)
AGENT_PID = 4242


class _Connection:
    def close(self) -> None:
        pass


def _mcp_call(data_root: str) -> list[str]:
    return [
        "--data-root",
        data_root,
        "--machine",
        "mcp",
        "call",
        "--tool",
        "status",
        "--arguments",
        "{}",
        "--workspace",
        "workspace-a",
        "--profile",
        "book-a",
    ]


class AgentSessionServiceTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = self._tmp.name
        opener = mock.patch("kassiber.operator.service.open_db", return_value=_Connection())
        opener.start()
        self.addCleanup(opener.stop)
        self.service = OperatorService("generation", lambda *_args: OperationResult(0, "{}", ""))
        self.addCleanup(self.service.close)

    def _grant(self, *, idle_timeout: int = 900) -> dict[str, object]:
        return self.service.unlock(
            self.root,
            bytearray(b"passphrase"),
            duration_seconds=3600,
            capability=Capability.READ,
            only_if_locked=True,
            agent_scope=True,
            idle_timeout_seconds=idle_timeout,
            agent_control=bytearray(CONTROL),
        )

    def _submit(
        self,
        *,
        pid: int | None = AGENT_PID,
        argv: list[str] | None = None,
        token: str | None = None,
    ) -> dict[str, object]:
        return self.service.submit(
            self.root,
            argv or _mcp_call(self.root),
            peer_pid=pid,
            agent_session_token=token,
        )

    def _reason(self, raised: AppError) -> object:
        return (raised.details or {}).get("reason")

    def test_an_agent_lease_is_read_only_and_needs_a_control_secret_and_idle_timeout(self) -> None:
        cases = [
            {"capability": Capability.OPERATOR, "idle_timeout_seconds": 900, "agent_control": bytearray(CONTROL)},
            {"capability": Capability.READ, "idle_timeout_seconds": 900, "agent_control": bytearray(b"short")},
            {"capability": Capability.READ, "idle_timeout_seconds": 30, "agent_control": bytearray(CONTROL)},
        ]
        for case in cases:
            with self.subTest(case=case), self.assertRaises(AppError):
                self.service.unlock(
                    self.root,
                    bytearray(b"passphrase"),
                    duration_seconds=3600,
                    agent_scope=True,
                    **case,
                )
        with self.assertRaises(AppError):
            # Terminal leases take neither.
            self.service.unlock(
                self.root,
                bytearray(b"passphrase"),
                duration_seconds=None,
                idle_timeout_seconds=900,
            )

    def test_an_unknown_process_is_asked_to_pair_and_nothing_runs(self) -> None:
        self._grant()
        with self.assertRaises(AppError) as raised:
            self._submit()
        self.assertEqual(raised.exception.code, "interaction_required")
        self.assertEqual(self._reason(raised.exception), "agent_session_required")

        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        with self.assertRaises(AppError) as raised:
            self._submit(token=session.token)
        self.assertEqual(self._reason(raised.exception), "agent_pairing_required")
        self.assertIn("allow this agent in Kassiber", raised.exception.hint or "")
        status = self.service.status(self.root)
        self.assertTrue(status["agent_scope"])
        self.assertEqual(
            [(item["id"], item["state"], item["label"]) for item in status["agent"]["sessions"]],
            [(session.id, "pending", "claude")],
        )
        self.assertEqual(self.service._operations, {})

    def test_only_the_control_secret_holder_can_allow_an_agent(self) -> None:
        self._grant()
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        with self.assertRaises(AppError) as raised:
            self.service.decide_agent_session(
                self.root, session.id, allow=True, control=bytearray(b"x" * 32)
            )
        self.assertEqual(raised.exception.code, "agent_pairing_unauthorized")
        with self.assertRaises(AppError):
            self._submit()

    def test_an_allowed_process_reads_and_its_activity_is_counted(self) -> None:
        self._grant()
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        decided = self.service.decide_agent_session(
            self.root, session.id, allow=True, control=bytearray(CONTROL)
        )
        self.assertEqual(decided["state"], "allowed")
        self.assertNotIn("token", decided)

        accepted = self._submit(token=session.token)

        self.assertIn(accepted["state"], {"queued", "running", "completed"})
        agent = self.service.status(self.root)["agent"]
        self.assertEqual(agent["calls"], 1)
        self.assertEqual(agent["sessions"][0]["calls"], 1)
        self.assertIsNotNone(agent["sessions"][0]["last_call_at"])
        # Another process, even with the token or the same label, is not
        # that session; nor is the same process id without the token (a
        # later process reusing it never received it).
        for other in ({"pid": AGENT_PID + 1, "token": session.token}, {"pid": AGENT_PID}, {"pid": AGENT_PID, "token": "x"}):
            with self.subTest(other=other), self.assertRaises(AppError) as raised:
                self._submit(**other)
            self.assertEqual(self._reason(raised.exception), "agent_session_required")

    def test_an_agent_lease_admits_only_mcp_tool_calls(self) -> None:
        self._grant()
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        self.service.decide_agent_session(self.root, session.id, allow=True, control=bytearray(CONTROL))
        with self.assertRaises(AppError) as raised:
            self._submit(
                argv=[
                    "--data-root",
                    self.root,
                    "--machine",
                    "transactions",
                    "list",
                    "--workspace",
                    "workspace-a",
                    "--profile",
                    "book-a",
                ]
            )
        self.assertEqual(raised.exception.code, "agent_session_scope")

    def test_denying_withdraws_and_cannot_be_undone_for_that_process(self) -> None:
        self._grant()
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        self.service.decide_agent_session(self.root, session.id, allow=True, control=bytearray(CONTROL))
        denied = self.service.decide_agent_session(
            self.root, session.id, allow=False, control=bytearray(CONTROL)
        )
        self.assertEqual(denied["state"], "denied")
        # The session closes (its connection ends at the next heartbeat) ...
        self.assertFalse(self.service.agent_session_alive(session.id))
        self.assertEqual(self.service.status(self.root)["agent"]["sessions"], [])
        with self.assertRaises(AppError) as raised:
            self.service.decide_agent_session(
                self.root, session.id, allow=True, control=bytearray(CONTROL)
            )
        self.assertEqual(raised.exception.code, "agent_session_not_found")
        # ... and the process stays refused, without a new request to allow.
        with self.assertRaises(AppError) as raised:
            self._submit()
        self.assertEqual(raised.exception.code, "agent_pairing_denied")
        with self.assertRaises(AppError) as raised:
            self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        self.assertEqual(raised.exception.code, "agent_pairing_denied")

    def test_denying_a_second_registration_withdraws_the_first_approval(self) -> None:
        self._grant()
        first = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        self.service.decide_agent_session(self.root, first.id, allow=True, control=bytearray(CONTROL))
        self._submit(token=first.token)
        second = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")

        self.service.decide_agent_session(self.root, second.id, allow=False, control=bytearray(CONTROL))

        self.assertFalse(self.service.agent_session_alive(first.id))
        self.assertFalse(self.service.agent_session_alive(second.id))
        with self.assertRaises(AppError) as raised:
            self._submit(token=first.token)
        self.assertEqual(raised.exception.code, "agent_pairing_denied")

    def test_denying_frees_a_session_slot(self) -> None:
        from kassiber.operator.service import MAX_AGENT_SESSIONS_PER_PROJECT

        self._grant()
        sessions = [
            self.service.open_agent_session(self.root, pid=AGENT_PID + offset, label=None)
            for offset in range(MAX_AGENT_SESSIONS_PER_PROJECT)
        ]
        self.service.decide_agent_session(
            self.root, sessions[0].id, allow=False, control=bytearray(CONTROL)
        )
        self.service.open_agent_session(self.root, pid=AGENT_PID + 99, label=None)

    def test_denials_are_never_forgotten_while_the_grant_lasts(self) -> None:
        from kassiber.operator import service as service_module

        self._grant()
        with mock.patch.object(service_module, "MAX_REMEMBERED_AGENT_DENIALS", 2):
            for offset in range(2):
                session = self.service.open_agent_session(self.root, pid=AGENT_PID + offset, label=None)
                self.service.decide_agent_session(
                    self.root, session.id, allow=False, control=bytearray(CONTROL)
                )
            with self.assertRaises(AppError) as raised:
                self.service.open_agent_session(self.root, pid=AGENT_PID + 50, label=None)
            self.assertEqual(raised.exception.code, "agent_session_limit")
            with self.assertRaises(AppError) as raised:
                self.service.open_agent_session(self.root, pid=AGENT_PID, label=None)
            self.assertEqual(raised.exception.code, "agent_pairing_denied")

    def test_a_denial_does_not_outlive_its_grant(self) -> None:
        self._grant()
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label=None)
        self.service.decide_agent_session(self.root, session.id, allow=False, control=bytearray(CONTROL))
        self.service.lock(self.root)
        self._grant()
        again = self.service.open_agent_session(self.root, pid=AGENT_PID, label=None)
        self.assertEqual(again.state, "pending")

    def test_a_closed_session_or_a_new_lease_needs_approval_again(self) -> None:
        self._grant()
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        self.service.decide_agent_session(self.root, session.id, allow=True, control=bytearray(CONTROL))
        self.service.close_agent_session(session.id)
        with self.assertRaises(AppError) as raised:
            self._submit()
        self.assertEqual(self._reason(raised.exception), "agent_session_required")

        again = self.service.open_agent_session(self.root, pid=AGENT_PID, label="claude")
        self.service.decide_agent_session(self.root, again.id, allow=True, control=bytearray(CONTROL))
        self.service.lock(self.root)
        self.assertFalse(self.service.agent_session_alive(again.id))
        self._grant()
        # The same process id under a new grant is a stranger again.
        with self.assertRaises(AppError) as raised:
            self._submit()
        self.assertEqual(self._reason(raised.exception), "agent_session_required")

    def test_sessions_need_an_agent_lease_and_a_known_process(self) -> None:
        with self.assertRaises(AppError) as raised:
            self.service.open_agent_session(self.root, pid=AGENT_PID, label=None)
        self.assertEqual(raised.exception.code, "interaction_required")
        self.service.unlock(self.root, bytearray(b"passphrase"), duration_seconds=None)
        with self.assertRaises(AppError) as raised:
            self.service.open_agent_session(self.root, pid=AGENT_PID, label=None)
        self.assertEqual(raised.exception.code, "agent_session_not_needed")
        self.service.lock(self.root)
        self._grant()
        with self.assertRaises(AppError) as raised:
            self.service.open_agent_session(self.root, pid=None, label=None)
        self.assertEqual(raised.exception.code, "agent_pairing_unavailable")

    def test_waiting_sessions_are_capped(self) -> None:
        from kassiber.operator.service import MAX_AGENT_SESSIONS_PER_PROJECT

        self._grant()
        for offset in range(MAX_AGENT_SESSIONS_PER_PROJECT):
            self.service.open_agent_session(self.root, pid=AGENT_PID + offset, label=None)
        with self.assertRaises(AppError) as raised:
            self.service.open_agent_session(self.root, pid=AGENT_PID + 99, label=None)
        self.assertEqual(raised.exception.code, "agent_session_limit")

    def test_an_idle_agent_lease_expires(self) -> None:
        self._grant(idle_timeout=60)
        lease = next(iter(self.service._leases.values()))
        self.assertFalse(lease.expired())
        lease.last_used_monotonic = time.monotonic() - 61
        self.assertTrue(lease.expired())
        self.assertEqual(self.service.status(self.root)["lease"], "locked")

    def test_an_approval_counts_as_activity(self) -> None:
        self._grant(idle_timeout=60)
        lease = next(iter(self.service._leases.values()))
        session = self.service.open_agent_session(self.root, pid=AGENT_PID, label=None)
        lease.last_used_monotonic = time.monotonic() - 50
        self.service.decide_agent_session(self.root, session.id, allow=True, control=bytearray(CONTROL))
        self.assertLess(time.monotonic() - lease.last_used_monotonic, 5)


@unittest.skipUnless(sys.platform.startswith("linux"), "SO_PEERCRED process id")
class PeerProcessTest(unittest.TestCase):
    def test_the_os_reports_the_connecting_process(self) -> None:
        from kassiber.operator.protocol import _verify_unix_peer

        left, right = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            self.assertEqual(_verify_unix_peer(left), os.getpid())
        finally:
            left.close()
            right.close()


class ProcessLabelTest(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith("linux"), "/proc process tree")
    def test_the_label_names_the_program_above_launchers(self) -> None:
        from kassiber.operator import peers

        names = {10: "kassiber", 9: "uv", 8: "claude"}
        parents = {11: 10, 10: 9, 9: 8, 8: 1}
        with mock.patch.object(peers, "_parent", side_effect=lambda pid: parents[pid]), mock.patch.object(
            peers, "_name", side_effect=lambda pid: names[pid]
        ):
            self.assertEqual(peers.process_label(11), "claude")

    def test_a_missing_process_has_no_label(self) -> None:
        from kassiber.operator import peers

        with mock.patch.object(peers, "_parent", side_effect=OSError("gone")):
            self.assertIsNone(peers.process_label(12345))
        self.assertIsNone(peers.process_label(None))


if __name__ == "__main__":
    unittest.main()


class AgentSessionServerTest(unittest.TestCase):
    def _server(self):
        from kassiber.operator.server import BrokerServer

        server = BrokerServer.__new__(BrokerServer)
        server.generation = "generation"
        server.service = mock.Mock()
        server.service.open_agent_session.return_value = mock.Mock(id="s1", state="pending", token="tok")
        return server

    def _channel(self, frames):
        channel = mock.MagicMock()
        channel.peer_pid = AGENT_PID
        channel.receive_json.side_effect = frames
        return channel

    def test_a_session_lives_while_heartbeats_arrive_and_ends_with_the_process(self) -> None:
        from kassiber.operator.protocol import PROTOCOL_VERSION

        server = self._server()
        server.service.agent_session_alive.return_value = True
        heartbeat = {"version": PROTOCOL_VERSION, "action": "agent_session_heartbeat"}
        channel = self._channel(
            [
                {"version": PROTOCOL_VERSION, "action": "agent_session_open", "data_root": "/book"},
                heartbeat,
                heartbeat,
                EOFError("process exited"),
            ]
        )
        with mock.patch("kassiber.operator.server._canonical_data_root", return_value="/book"), mock.patch(
            "kassiber.operator.peers.process_label", return_value="claude"
        ):
            server._serve_channel(channel)

        server.service.open_agent_session.assert_called_once_with("/book", pid=AGENT_PID, label="claude")
        sent = channel.send_json.call_args_list
        self.assertEqual(len(sent), 1)
        self.assertEqual(
            sent[0].args[0]["data"], {"session_id": "s1", "state": "pending", "token": "tok"}
        )
        server.service.close_agent_session.assert_called_once_with("s1")
        self.assertEqual(channel.receive_json.call_count, 4)

    def test_a_session_ends_on_any_other_frame_or_when_its_lease_ends(self) -> None:
        from kassiber.operator.protocol import PROTOCOL_VERSION

        server = self._server()
        server.service.agent_session_alive.side_effect = [True, False]
        channel = self._channel(
            [
                {"version": PROTOCOL_VERSION, "action": "agent_session_open", "data_root": "/book"},
                {"version": PROTOCOL_VERSION, "action": "agent_session_heartbeat"},
                AssertionError("no read after the lease ended"),
            ]
        )
        with mock.patch("kassiber.operator.server._canonical_data_root", return_value="/book"), mock.patch(
            "kassiber.operator.peers.process_label", return_value=None
        ):
            server._serve_channel(channel)
        server.service.close_agent_session.assert_called_once_with("s1")

    def test_a_refused_session_gets_an_error_and_holds_nothing(self) -> None:
        from kassiber.operator.protocol import PROTOCOL_VERSION

        server = self._server()
        server.service.open_agent_session.side_effect = AppError("no", code="agent_session_not_needed")
        channel = self._channel(
            [{"version": PROTOCOL_VERSION, "action": "agent_session_open", "data_root": "/book"}]
        )
        with mock.patch("kassiber.operator.server._canonical_data_root", return_value="/book"), mock.patch(
            "kassiber.operator.peers.process_label", return_value=None
        ):
            server._serve_channel(channel)
        response = channel.send_json.call_args.args[0]
        self.assertFalse(response["ok"])
        self.assertEqual(response["error"]["code"], "agent_session_not_needed")
        server.service.close_agent_session.assert_not_called()

    def test_decisions_need_the_control_secret_frame(self) -> None:
        server = self._server()
        server.service.decide_agent_session.return_value = {"id": "s1", "state": "allowed"}
        channel = mock.MagicMock()
        channel.receive_secret.return_value = bytearray(CONTROL)
        with mock.patch("kassiber.operator.server._canonical_data_root", return_value="/book"):
            response = server._handle(
                channel,
                {"action": "agent_session_decide", "data_root": "/book", "session_id": "s1", "allow": True},
            )
        self.assertEqual(response["data"]["state"], "allowed")
        continuation = channel.send_json.call_args.args[0]
        self.assertEqual(continuation["continue"], "secret")
        self.assertEqual(continuation["label"], "agent_control")
        _, kwargs = server.service.decide_agent_session.call_args
        self.assertTrue(kwargs["allow"])
        with self.assertRaises(AppError):
            server._handle(
                channel,
                {"action": "agent_session_decide", "data_root": "/book", "session_id": "s1", "allow": "yes"},
            )
