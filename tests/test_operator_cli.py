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
from kassiber.operator.cli import route_brokered_command


class OperatorCliTest(unittest.TestCase):
    def test_brokered_mode_does_not_bypass_queue_for_passphrase_fd(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            read_fd, write_fd = os.pipe()
            os.write(write_fd, b"database-passphrase")
            os.close(write_fd)
            args = mock.Mock(
                command="status",
                db_passphrase_fd=read_fd,
                data_root=tmp,
                env_file=None,
                project=None,
                operator_auth_fd=None,
                non_interactive=True,
            )
            captured: dict[str, object] = {}

            def submit(_client, data_root, prepared, *, admin_authentication):
                captured["data_root"] = data_root
                captured["argv"] = list(prepared.argv)
                captured["secrets"] = {
                    label: bytes(value) for label, value in prepared.secrets.items()
                }
                return {"operation_id": "generation.operation", "state": "queued"}

            try:
                with mock.patch(
                    "kassiber.operator.cli.effective_unlock_mode",
                    return_value="brokered",
                ), mock.patch(
                    "kassiber.cli.command_registry.command_path",
                    return_value="status",
                ), mock.patch(
                    "kassiber.operator.cli.BrokerClient.submit",
                    autospec=True,
                    side_effect=submit,
                ), mock.patch(
                    "kassiber.operator.cli.BrokerClient.wait",
                    return_value={"state": "completed", "exit_code": 0},
                ):
                    exit_code = route_brokered_command(
                        args,
                        ["--db-passphrase-fd", str(read_fd), "status"],
                    )
            finally:
                try:
                    os.close(read_fd)
                except OSError:
                    pass

        self.assertEqual(exit_code, 0)
        self.assertEqual(captured["data_root"], tmp)
        self.assertNotIn(str(read_fd), captured["argv"])
        self.assertEqual(list(captured["secrets"].values()), [b"database-passphrase"])

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
