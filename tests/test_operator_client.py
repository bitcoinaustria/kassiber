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
        channel = mock.MagicMock()
        channel.__enter__.return_value = channel
        channel.receive_json.return_value = {
            "ok": True,
            "data": {"operation_id": "generation.client.op", "state": "queued"},
        }
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
            request = channel.send_json.call_args.args[0]
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
