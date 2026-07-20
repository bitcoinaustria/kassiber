from __future__ import annotations

import contextlib
import io
import json
import os
import tempfile
import unittest
from unittest import mock

from kassiber.cli.main import main
from kassiber.cli.main import _verify_operator_child_open_database
from kassiber.core.runtime import _operator_expected_database_identity
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.operator.protocol import TEST_RUNTIME_OVERRIDE_ENV


class OperatorCliTest(unittest.TestCase):
    def test_worker_requires_project_binding_before_runtime_bootstrap(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ,
            {"KASSIBER_OPERATOR_CHILD": "1"},
            clear=True,
        ):
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(["--data-root", tmp, "--machine", "status"])

        self.assertEqual(exit_code, 1)
        payload = json.loads(stdout.getvalue())
        self.assertEqual(
            payload["error"]["code"],
            "operator_project_binding_invalid",
        )

    def test_worker_requires_database_binding_in_every_runtime_open(self) -> None:
        with mock.patch.dict(
            os.environ,
            {"KASSIBER_OPERATOR_CHILD": "1"},
            clear=True,
        ):
            with self.assertRaises(AppError) as raised:
                _operator_expected_database_identity()
        self.assertEqual(
            raised.exception.code,
            "operator_project_binding_invalid",
        )

    def test_worker_open_verification_requires_database_binding(self) -> None:
        with mock.patch.dict(
            os.environ,
            {"KASSIBER_OPERATOR_CHILD": "1"},
            clear=True,
        ):
            with self.assertRaises(AppError) as raised:
                _verify_operator_child_open_database(None)
        self.assertEqual(
            raised.exception.code,
            "operator_project_binding_invalid",
        )

    def test_worker_rejects_the_database_connection_it_did_not_admit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ,
            {"KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY": "0" * 32},
        ):
            connection = open_db(tmp)
            try:
                with self.assertRaises(AppError) as raised:
                    _verify_operator_child_open_database(connection)
                self.assertEqual(raised.exception.code, "operator_project_replaced")
            finally:
                connection.close()

    def test_worker_rejects_replacement_before_open_db_migrations(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            connection = open_db(tmp)
            connection.close()
            with self.assertRaises(AppError) as raised:
                open_db(tmp, expected_database_identity="0" * 32)
            self.assertEqual(raised.exception.code, "operator_project_replaced")

    def test_machine_unlock_never_prompts_or_starts_broker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.cli.prompt_passphrase"
        ) as prompt:
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--data-root",
                        tmp,
                        "--machine",
                        "operator",
                        "unlock",
                    ]
                )
            self.assertEqual(exit_code, 1)
            prompt.assert_not_called()
            payload = json.loads(stdout.getvalue())
            self.assertEqual(payload["error"]["code"], "interaction_required")

    def test_status_is_public_safe_and_does_not_start_broker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as runtime, mock.patch.dict(
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": runtime,
                TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
        ):
            os.chmod(runtime, 0o700)
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--data-root",
                        tmp,
                        "--machine",
                        "operator",
                        "status",
                    ]
                )
            self.assertEqual(exit_code, 0)
            payload = json.loads(stdout.getvalue())
            self.assertEqual(payload["kind"], "operator.status")
            self.assertEqual(payload["data"]["broker"], "stopped")
            self.assertNotIn(tmp, stdout.getvalue())

    def test_touch_id_status_is_truthful_without_native_helper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--data-root",
                        tmp,
                        "--machine",
                        "operator",
                        "touch-id",
                        "status",
                    ]
                )
            self.assertEqual(exit_code, 0)
            payload = json.loads(stdout.getvalue())
            self.assertFalse(payload["data"]["available"])
            self.assertEqual(payload["data"]["reason"], "native_auth_unavailable")

    def test_machine_touch_id_unlock_never_invokes_native_helper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.cli.BrokerClient.unlock_touch_id"
        ) as unlock_touch_id:
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(
                    [
                        "--data-root",
                        tmp,
                        "--machine",
                        "operator",
                        "unlock",
                        "--auth",
                        "touch-id",
                    ]
                )
            self.assertEqual(exit_code, 1)
            unlock_touch_id.assert_not_called()
            payload = json.loads(stdout.getvalue())
            self.assertEqual(payload["error"]["code"], "interaction_required")


if __name__ == "__main__":
    unittest.main()
