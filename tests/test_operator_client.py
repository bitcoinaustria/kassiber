from __future__ import annotations

import unittest
from unittest import mock

from kassiber.errors import AppError
from kassiber.operator.client import (
    BrokerClient,
    prepare_arguments,
    wipe_prepared,
)


class OperatorClientSubmitTest(unittest.TestCase):
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
