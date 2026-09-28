from __future__ import annotations

import io
import os
import tempfile
import unittest
from unittest import mock

from kassiber.errors import AppError
from kassiber.operator.client import (
    MAX_CLIENT_SECRET_BYTES,
    BrokerClient,
    PreparedArguments,
    prepare_arguments,
    wipe_prepared,
)
from kassiber.operator.protocol import MAX_SECRET_PAYLOAD_BYTES
from kassiber.secrets.prompt import MAX_PASSPHRASE_BYTES, read_passphrase_from_fd


class OperatorClientSubmitTest(unittest.TestCase):
    def test_prepare_arguments_respects_child_and_wire_secret_limits(self) -> None:
        self.assertEqual(MAX_CLIENT_SECRET_BYTES, MAX_PASSPHRASE_BYTES)
        self.assertLessEqual(MAX_CLIENT_SECRET_BYTES, MAX_SECRET_PAYLOAD_BYTES)
        payload = b"s" * MAX_CLIENT_SECRET_BYTES
        prepared = prepare_arguments(
            ["backends", "create", "--token-stdin"],
            stdin=io.BytesIO(payload),
        )
        try:
            self.assertEqual(next(iter(prepared.secrets.values())), payload)
        finally:
            wipe_prepared(prepared)

        with tempfile.TemporaryFile() as child_input:
            child_input.write(payload)
            child_input.seek(0)
            self.assertEqual(
                read_passphrase_from_fd(os.dup(child_input.fileno())),
                payload.decode("utf-8"),
            )

        with self.assertRaises(AppError) as raised:
            prepare_arguments(
                ["backends", "create", "--token-stdin"],
                stdin=io.BytesIO(payload + b"s"),
            )
        self.assertEqual(raised.exception.code, "operator_secret_too_large")

    def test_signed_cli_restarts_an_idle_mismatched_helper_broker(self) -> None:
        client = BrokerClient()
        old = {
            "broker": "running",
            "generation": "old-generation",
            "native_auth_available": True,
            "native_auth_identity": "untrusted-helper",
        }
        replacement = {
            "broker": "running",
            "generation": "new-generation",
            "native_auth_available": True,
            "native_auth_identity": "signed-helper",
        }
        with mock.patch("kassiber.operator.client.sys.platform", "darwin"), mock.patch.dict(
            "kassiber.operator.client.os.environ",
            {"KASSIBER_NATIVE_AUTH_HELPER": "/signed/Kassiber"},
        ), mock.patch.object(
            client,
            "ensure_running",
            side_effect=[old, replacement],
        ) as ensure, mock.patch(
            "kassiber.operator.native_auth.native_auth_caller_identity",
            return_value="signed-helper",
        ), mock.patch.object(
            client,
            "_simple_request",
            return_value={"restart": "accepted"},
        ) as restart, mock.patch.object(
            client,
            "ping",
            side_effect=ConnectionRefusedError(),
        ):
            self.assertEqual(client.ensure_native_auth_running(), replacement)

        restart.assert_called_once_with("restart_for_native_auth")
        self.assertEqual(ensure.call_count, 2)

    def test_retry_app_error_from_new_broker_reports_result_unknown(self) -> None:
        client = BrokerClient()
        prepared = prepare_arguments(["status"])
        retry_error = AppError(
            "unlock required",
            code="interaction_required",
            retryable=False,
        )
        try:
            with mock.patch.object(
                client,
                "ensure_running",
                return_value={"generation": "old-generation"},
            ), mock.patch.object(
                client,
                "_submit_once",
                side_effect=[ConnectionResetError(), retry_error],
            ), mock.patch.object(
                client,
                "operation_status",
                return_value={
                    "operation_id": "old-generation.client.fixed-operation",
                    "state": "result_unknown",
                    "reason": "broker_generation_changed",
                },
            ), mock.patch(
                "kassiber.operator.client.secrets.token_hex",
                return_value="fixed-operation",
            ):
                with self.assertRaises(AppError) as raised:
                    client.submit(
                        "/project",
                        prepared,
                        admin_authentication=None,
                    )
            self.assertEqual(
                raised.exception.code,
                "operator_submission_result_unknown",
            )
            self.assertFalse(raised.exception.retryable)
            self.assertEqual(
                raised.exception.details,
                {
                    "operation_id": "old-generation.client.fixed-operation",
                    "state": "result_unknown",
                    "reason": "broker_generation_changed",
                },
            )
        finally:
            wipe_prepared(prepared)

    def test_retry_app_error_is_unknown_when_status_is_not_retained(self) -> None:
        client = BrokerClient()
        prepared = prepare_arguments(["status"])
        retry_error = AppError(
            "command rejected",
            code="operator_capability_denied",
            retryable=False,
        )
        try:
            with mock.patch.object(
                client,
                "ensure_running",
                return_value={"generation": "generation"},
            ), mock.patch.object(
                client,
                "_submit_once",
                side_effect=[ConnectionResetError(), retry_error],
            ), mock.patch.object(
                client,
                "operation_status",
                return_value={
                    "operation_id": "generation.client.fixed-operation",
                    "state": "result_unknown",
                    "reason": "result_not_retained",
                },
            ), mock.patch(
                "kassiber.operator.client.secrets.token_hex",
                return_value="fixed-operation",
            ):
                with self.assertRaises(AppError) as raised:
                    client.submit(
                        "/project",
                        prepared,
                        admin_authentication=None,
                    )
            self.assertEqual(
                raised.exception.code,
                "operator_submission_result_unknown",
            )
            self.assertEqual(
                raised.exception.details,
                {
                    "operation_id": "generation.client.fixed-operation",
                    "state": "result_unknown",
                    "reason": "result_not_retained",
                },
            )
        finally:
            wipe_prepared(prepared)

    def test_retry_error_returns_status_when_original_operation_is_known(self) -> None:
        client = BrokerClient()
        prepared = prepare_arguments(["status"])
        known = {
            "operation_id": "generation.client.fixed-operation",
            "state": "completed",
            "exit_code": 0,
        }
        try:
            with mock.patch.object(
                client,
                "ensure_running",
                return_value={"generation": "generation"},
            ), mock.patch.object(
                client,
                "_submit_once",
                side_effect=[ConnectionResetError(), AppError("lease ended")],
            ), mock.patch.object(
                client,
                "operation_status",
                return_value=known,
            ), mock.patch(
                "kassiber.operator.client.secrets.token_hex",
                return_value="fixed-operation",
            ):
                self.assertEqual(
                    client.submit(
                        "/project",
                        prepared,
                        admin_authentication=None,
                    ),
                    known,
                )
        finally:
            wipe_prepared(prepared)

    def test_unverifiable_generation_reports_result_unknown(self) -> None:
        client = BrokerClient()
        prepared = prepare_arguments(["status"])
        try:
            with mock.patch.object(
                client,
                "ensure_running",
                return_value={"generation": "generation"},
            ), mock.patch.object(
                client,
                "_submit_once",
                side_effect=[ConnectionResetError(), ConnectionRefusedError()],
            ), mock.patch.object(
                client,
                "operation_status",
                return_value={
                    "operation_id": "generation.client.fixed-operation",
                    "state": "result_unknown",
                    "reason": "broker_unreachable",
                },
            ), mock.patch(
                "kassiber.operator.client.secrets.token_hex",
                return_value="fixed-operation",
            ):
                with self.assertRaises(AppError) as raised:
                    client.submit(
                        "/project",
                        prepared,
                        admin_authentication=None,
                    )
            self.assertEqual(
                raised.exception.details,
                {
                    "operation_id": "generation.client.fixed-operation",
                    "state": "result_unknown",
                    "reason": "broker_unreachable",
                },
            )
        finally:
            wipe_prepared(prepared)


if __name__ == "__main__":
    unittest.main()


class OperatorClientCallerContextTest(unittest.TestCase):
    def _submitted_request(self, environment: dict[str, str]) -> dict[str, object]:
        from kassiber.operator.build import build_identity

        channel = mock.MagicMock()
        channel.__enter__.return_value = channel
        channel.receive_json.side_effect = [
            {"ok": True, "continue": "argv", "build": build_identity()},
            {"ok": True, "data": {"operation_id": "generation.client.op", "state": "queued"}},
        ]
        client = BrokerClient()
        prepared = prepare_arguments(["--output", "relative.json", "status"])
        with tempfile.TemporaryDirectory() as caller_directory:
            previous = os.getcwd()
            os.chdir(caller_directory)
            try:
                with mock.patch.dict(os.environ, environment, clear=False), mock.patch(
                    "kassiber.operator.client.connect",
                    return_value=channel,
                ):
                    client._submit_once(
                        "/project",
                        prepared,
                        operation_id="generation.client.op",
                        admin_authentication=None,
                    )
            finally:
                os.chdir(previous)
            request = channel.send_json.call_args_list[0].args[0]
            # The command line waits for the broker to name its build.
            self.assertNotIn("argv", request)
            self.assertEqual(
                channel.send_json.call_args_list[1].args[0],
                {"argv": ["--output", "relative.json", "status"]},
            )
            self.assertEqual(
                os.path.realpath(str(request["working_directory"])),
                os.path.realpath(caller_directory),
            )
        return request

    def test_submit_carries_the_callers_working_directory(self) -> None:
        request = self._submitted_request({"KASSIBER_NO_EGRESS": ""})
        self.assertIs(request["no_egress"], False)

    def test_submit_carries_the_callers_egress_kill_switch(self) -> None:
        request = self._submitted_request({"KASSIBER_NO_EGRESS": "yes"})
        self.assertIs(request["no_egress"], True)


class OperatorClientOutdatedBrokerTest(unittest.TestCase):
    def _submit(self, environment: dict[str, str]) -> mock.Mock:
        client = BrokerClient()
        prepared = prepare_arguments(["status"])
        submit_once = mock.Mock(return_value={"operation_id": "g.client.x", "state": "queued"})
        with mock.patch.dict(os.environ, environment, clear=False), mock.patch.object(
            client, "ensure_running", return_value={"generation": "g"}
        ), mock.patch.object(client, "_submit_once", submit_once):
            try:
                client.submit("/project", prepared, admin_authentication=None)
            finally:
                wipe_prepared(prepared)
        return submit_once

    def test_egress_kill_switch_fails_closed_against_an_older_broker(self) -> None:
        with self.assertRaises(AppError) as raised:
            self._submit({"KASSIBER_NO_EGRESS": "1"})
        self.assertEqual(raised.exception.code, "operator_broker_outdated")

    def test_older_broker_without_kill_switch_submits_with_a_warning(self) -> None:
        stderr = io.StringIO()
        with mock.patch("sys.stderr", stderr):
            submit_once = self._submit({"KASSIBER_NO_EGRESS": ""})
        submit_once.assert_called_once()
        self.assertIn("absolute paths", stderr.getvalue())


class OperatorClientUnreadableDirectoryTest(unittest.TestCase):
    def test_an_unreadable_cwd_fails_instead_of_using_the_brokers(self) -> None:
        from kassiber.operator import client as client_module

        with mock.patch.object(
            client_module.os, "getcwd", side_effect=FileNotFoundError()
        ):
            with self.assertRaises(AppError) as raised:
                client_module._caller_working_directory()
        self.assertEqual(raised.exception.code, "operator_working_directory_unavailable")


class OperatorBrokerLaunchDirectoryTest(unittest.TestCase):
    def test_broker_starts_in_the_package_location_not_the_callers_directory(self) -> None:
        from kassiber.operator import client as client_module

        client = BrokerClient()
        with tempfile.TemporaryDirectory() as caller:
            previous = os.getcwd()
            os.chdir(caller)
            try:
                with mock.patch.object(
                    client, "ping", side_effect=[ConnectionRefusedError(), {"generation": "g"}]
                ), mock.patch.object(client_module.subprocess, "Popen") as popen:
                    client.ensure_running()
            finally:
                os.chdir(previous)
        cwd = popen.call_args.kwargs["cwd"]
        self.assertNotEqual(os.path.realpath(cwd), os.path.realpath(caller))
        self.assertTrue(os.path.isdir(os.path.join(cwd, "kassiber")))


class OperatorClientBuildBindingTest(unittest.TestCase):
    def test_unlock_never_sends_a_passphrase_to_another_builds_broker(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        other = {**build_identity(), "origin": "f" * 12}
        with mock.patch.object(
            client, "ensure_running", return_value={"generation": "g", "build": other}
        ), mock.patch(
            "kassiber.operator.client.connect",
            side_effect=AssertionError("must not connect to send the passphrase"),
        ):
            with self.assertRaises(AppError) as raised:
                client.unlock(
                    "/project",
                    bytearray(b"secret"),
                    duration_seconds=None,
                    capability="read",
                    authentication_method="password",
                )
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")
        self.assertEqual(raised.exception.details["broker_build"], other)

    def test_a_broker_without_build_identity_is_another_build(self) -> None:
        client = BrokerClient()
        with mock.patch.object(client, "ensure_running", return_value={"generation": "g"}):
            with self.assertRaises(AppError) as raised:
                client.set_mode("/project", "brokered", bytearray(b"secret"))
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")

    def test_protocol_mismatch_fails_fast_without_spawning_a_broker(self) -> None:
        client = BrokerClient()
        mismatch = AppError("mismatch", code="operator_protocol_version_mismatch")
        with mock.patch.object(client, "ping", side_effect=mismatch), mock.patch(
            "kassiber.operator.client.subprocess.Popen",
            side_effect=AssertionError("must not spawn"),
        ):
            with self.assertRaises(AppError) as raised:
                client.ensure_running()
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")

    def test_stop_asks_an_idle_broker_to_exit_and_explains_a_busy_one(self) -> None:
        client = BrokerClient()
        with mock.patch.object(client, "ping", return_value={"build": {"version": "x"}}), mock.patch.object(
            client, "_simple_request", return_value={"restart": "accepted"}
        ) as request:
            self.assertEqual(client.stop()["broker"], "stopping")
        request.assert_called_once_with("restart_for_native_auth")
        busy = AppError("busy", code="operator_broker_busy")
        with mock.patch.object(client, "ping", return_value={}), mock.patch.object(
            client, "_simple_request", side_effect=busy
        ):
            with self.assertRaises(AppError) as raised:
                client.stop()
        self.assertIn("operator lock", raised.exception.hint)
        with mock.patch.object(client, "ping", side_effect=ConnectionRefusedError()):
            self.assertEqual(client.stop(), {"broker": "stopped"})

    def test_stop_explains_a_broker_of_another_protocol(self) -> None:
        client = BrokerClient()
        mismatch = AppError("mismatch", code="operator_protocol_version_mismatch")
        with mock.patch.object(client, "ping", side_effect=mismatch), mock.patch.object(
            client, "_simple_request", side_effect=AssertionError("it would refuse this too")
        ):
            with self.assertRaises(AppError) as raised:
                client.stop()
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")
        self.assertIn("from that build", raised.exception.hint)

    def test_the_broker_asking_for_the_secret_must_be_this_build(self) -> None:
        # The ping reached this build, but another broker took the endpoint
        # before the secret-bearing connection.
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        other = {**build_identity(), "origin": "f" * 12}
        channel = _ScriptedChannel(
            [{"ok": True, "continue": "secret", "challenge": "c", "build": other}]
        )
        with mock.patch.object(
            client, "ensure_running", return_value={"generation": "g", "build": build_identity()}
        ), mock.patch("kassiber.operator.client.connect", return_value=channel):
            for call in (
                lambda: client.unlock(
                    "/project",
                    bytearray(b"secret"),
                    duration_seconds=None,
                    capability="read",
                    authentication_method="password",
                ),
                lambda: client.set_mode("/project", "brokered", bytearray(b"secret")),
            ):
                channel.responses[:] = [
                    {"ok": True, "continue": "secret", "challenge": "c", "build": other}
                ]
                with self.subTest(call=call), self.assertRaises(AppError) as raised:
                    call()
                self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")
        self.assertEqual(channel.secrets, [])

    def test_secrets_never_go_to_another_build_even_for_ordinary_commands(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        other = {**build_identity(), "origin": "f" * 12}
        channel = _ScriptedChannel([{"ok": True, "continue": "argv", "build": other}])
        with mock.patch("kassiber.operator.client.connect", return_value=channel):
            with self.assertRaises(AppError) as raised:
                client._submit_once(
                    "/project",
                    PreparedArguments(["backends", "update", "x", "--token", "inline-secret"], {}),
                    operation_id="g.client.1",
                    admin_authentication=bytearray(b"fresh-passphrase"),
                )
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")
        self.assertEqual(channel.secrets, [])
        # Nothing but the opening request (no command line) reached it.
        self.assertEqual(len(channel.sent), 1)
        self.assertNotIn("inline-secret", repr(channel.sent))

    def test_work_another_build_accepted_is_withdrawn_when_this_build_is_required(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        other = {**build_identity(), "origin": "f" * 12}
        accepted = {"operation_id": "g.client.1", "state": "queued", "build": other}
        with mock.patch.object(
            client, "ping", return_value={"generation": "g", "build": build_identity(), "caller_context": True}
        ), mock.patch.object(client, "_submit_once", return_value=accepted), mock.patch.object(
            client, "cancel"
        ) as cancel:
            with self.assertRaises(AppError) as raised:
                client.submit(
                    "/project",
                    PreparedArguments(["status"], {}),
                    admin_authentication=None,
                    start_broker=False,
                    require_same_build=True,
                )
            self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")
            cancel.assert_called_once()
            # Ordinary commands run where the lease is, and say so.
            ordinary = client.submit(
                "/project",
                PreparedArguments(["status"], {}),
                admin_authentication=None,
                start_broker=False,
            )
        self.assertEqual(ordinary["broker_build"], other)
        self.assertNotIn("build", ordinary)

    def test_an_acceptance_without_a_build_falls_back_to_the_ping(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        other = {**build_identity(), "origin": "f" * 12}
        with mock.patch.object(
            client, "ping", return_value={"generation": "g", "build": other, "caller_context": True}
        ), mock.patch.object(
            client, "_submit_once", return_value={"operation_id": "g.client.1", "state": "queued"}
        ), mock.patch.object(client, "cancel") as cancel:
            ordinary = client.submit(
                "/project",
                PreparedArguments(["status"], {}),
                admin_authentication=None,
                start_broker=False,
            )
            self.assertEqual(ordinary["broker_build"], other)
            # Required binding never trusts the ping for an unnamed acceptance.
            with mock.patch.object(
                client,
                "ping",
                return_value={"generation": "g", "build": build_identity(), "caller_context": True},
            ), self.assertRaises(AppError):
                client.submit(
                    "/project",
                    PreparedArguments(["status"], {}),
                    admin_authentication=None,
                    start_broker=False,
                    require_same_build=True,
                )
        cancel.assert_called_once()

    def test_a_retry_accepted_by_another_build_is_still_refused(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        other = {**build_identity(), "origin": "f" * 12}
        accepted = {"operation_id": "g.client.1", "state": "queued", "build": other}
        with mock.patch.object(
            client, "ping", return_value={"generation": "g", "build": build_identity(), "caller_context": True}
        ), mock.patch.object(
            client, "_submit_once", side_effect=[ConnectionResetError(), accepted]
        ), mock.patch.object(client, "cancel"), mock.patch.object(
            client, "operation_status", side_effect=AssertionError("no reconciliation")
        ):
            with self.assertRaises(AppError) as raised:
                client.submit(
                    "/project",
                    PreparedArguments(["status"], {}),
                    admin_authentication=None,
                    start_broker=False,
                    require_same_build=True,
                )
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")

    def test_submit_asks_the_broker_to_refuse_another_build(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        channel = _ScriptedChannel(
            [
                {"ok": True, "continue": "argv", "build": build_identity()},
                {"ok": True, "data": {"operation_id": "g.client.1", "build": build_identity()}},
                {"ok": True, "continue": "argv", "build": build_identity()},
                {"ok": True, "data": {"operation_id": "g.client.2"}},
            ]
        )
        with mock.patch("kassiber.operator.client.connect", return_value=channel):
            for operation_id, required in (("g.client.1", True), ("g.client.2", False)):
                client._submit_once(
                    "/project",
                    PreparedArguments(["status"], {}),
                    operation_id=operation_id,
                    admin_authentication=None,
                    require_same_build=required,
                )
        self.assertEqual(channel.sent[0]["expected_build"], build_identity())
        self.assertNotIn("expected_build", channel.sent[2])

    def test_a_retried_unnamed_acceptance_reports_no_build(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        with mock.patch.object(
            client, "ping", return_value={"generation": "g", "build": build_identity(), "caller_context": True}
        ), mock.patch.object(
            client,
            "_submit_once",
            side_effect=[ConnectionResetError(), {"operation_id": "g.client.1", "state": "queued"}],
        ):
            result = client.submit(
                "/project",
                PreparedArguments(["status"], {}),
                admin_authentication=None,
                start_broker=False,
            )
        self.assertIsNone(result["broker_build"])

    def test_a_retried_submit_still_names_the_brokers_build(self) -> None:
        from kassiber.operator.build import build_identity

        client = BrokerClient()
        accepted = {"operation_id": "g.client.1", "state": "queued", "build": build_identity()}
        with mock.patch.object(
            client, "ping", return_value={"generation": "g", "build": build_identity(), "caller_context": True}
        ), mock.patch.object(
            client, "_submit_once", side_effect=[ConnectionResetError(), accepted]
        ):
            result = client.submit(
                "/project",
                PreparedArguments(["status"], {}),
                admin_authentication=None,
                start_broker=False,
                require_same_build=True,
            )
        self.assertEqual(result["broker_build"], build_identity())


class _ScriptedChannel:
    """A broker connection that replays responses and records secrets."""

    def __init__(self, responses: list[dict]) -> None:
        self.responses = responses
        self.secrets: list[bytes] = []
        self.sent: list[dict] = []

    def __enter__(self):
        return self

    def __exit__(self, *exc) -> None:
        return None

    def send_json(self, payload) -> None:
        self.sent.append(payload)

    def receive_json(self):
        return self.responses.pop(0)

    def send_secret(self, challenge, secret) -> None:
        self.secrets.append(bytes(secret))
