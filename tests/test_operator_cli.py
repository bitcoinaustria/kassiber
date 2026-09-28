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
from kassiber.core import accounts as core_accounts
from kassiber.core.runtime import _operator_expected_database_identity
from kassiber.core.ui_snapshot import build_workspace_health_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.operator.protocol import TEST_RUNTIME_OVERRIDE_ENV
from kassiber.operator.cli import route_brokered_command


class OperatorCliTest(unittest.TestCase):
    def test_update_is_global_and_never_routed_through_the_project_broker(self) -> None:
        args = mock.Mock(command="update")
        with mock.patch(
            "kassiber.operator.cli.effective_unlock_mode",
            side_effect=AssertionError("global update must not inspect a project"),
        ):
            self.assertIsNone(route_brokered_command(args, ["update"]))

    def test_verify_download_is_global_and_never_routed_through_the_project_broker(self) -> None:
        args = mock.Mock(command="verify-download")
        with mock.patch(
            "kassiber.operator.cli.effective_unlock_mode",
            side_effect=AssertionError("download verification must not inspect a project"),
        ):
            self.assertIsNone(route_brokered_command(args, ["verify-download"]))

    def test_manual_next_actions_preserves_new_project_guidance(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(
                    ["--data-root", tmp, "--machine", "next-actions"]
                )

        self.assertEqual(exit_code, 0)
        payload = json.loads(stdout.getvalue())
        self.assertEqual(
            payload["data"]["suggestions"][0]["id"],
            "create_workspace_profile",
        )

    def test_scoped_health_ignores_another_clients_current_context(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            connection = open_db(tmp)
            try:
                first_workspace = core_accounts.create_workspace(
                    connection,
                    "First book set",
                )
                first_profile = core_accounts.create_profile(
                    connection,
                    first_workspace["id"],
                    "First book",
                    "EUR",
                    "FIFO",
                    "generic",
                    365,
                )
                second_workspace = core_accounts.create_workspace(
                    connection,
                    "Second book set",
                )
                core_accounts.create_profile(
                    connection,
                    second_workspace["id"],
                    "Second book",
                    "EUR",
                    "FIFO",
                    "generic",
                    365,
                )

                health = build_workspace_health_snapshot(
                    connection,
                    workspace_id=first_workspace["id"],
                    profile_id=first_profile["id"],
                )
            finally:
                connection.close()

        self.assertEqual(health["workspace"]["id"], first_workspace["id"])
        self.assertEqual(health["profile"]["id"], first_profile["id"])

    def test_brokered_chat_fails_before_direct_database_open(self) -> None:
        args = mock.Mock(
            command="chat",
            data_root="/project",
            env_file=None,
            project=None,
        )
        with mock.patch(
            "kassiber.operator.cli._selected_data_root",
            return_value="/canonical-project",
        ), mock.patch(
            "kassiber.operator.cli.effective_unlock_mode",
            return_value="brokered",
        ), self.assertRaises(AppError) as raised:
            route_brokered_command(args, ["chat"])

        self.assertEqual(raised.exception.code, "operator_chat_not_supported")

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
                machine=True,
            )
            captured: dict[str, object] = {}
            stderr = io.StringIO()

            def submit(_client, data_root, prepared, *, admin_authentication, **options):
                captured["start_broker"] = options.get("start_broker", True)
                captured["data_root"] = data_root
                captured["argv"] = list(prepared.argv)
                captured["secrets"] = {
                    label: bytes(value) for label, value in prepared.secrets.items()
                }
                return {"operation_id": "generation.operation", "state": "queued"}

            try:
                with contextlib.redirect_stderr(stderr), mock.patch(
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
        self.assertNotIn("--db-passphrase-fd", captured["argv"])
        self.assertNotIn(str(read_fd), captured["argv"])
        self.assertEqual(captured["secrets"], {})
        # An ordinary command must never start a broker of its own build.
        self.assertIs(captured["start_broker"], False)
        # Machine mode writes exactly one accepted event and nothing secret.
        events = [json.loads(line) for line in stderr.getvalue().splitlines()]
        self.assertEqual(
            events,
            [
                {
                    "kind": "operator.operation.accepted",
                    "schema_version": 1,
                    "data": {
                        "operation_id": "generation.operation",
                        "state": "queued",
                        "broker_build": None,
                    },
                }
            ],
        )
        self.assertNotIn("database-passphrase", stderr.getvalue())

    def _route_machine_command(
        self,
        completed: dict[str, object],
    ) -> tuple[int | None, str, str]:
        args = mock.Mock(
            command="status",
            db_passphrase_fd=None,
            data_root="/project",
            env_file=None,
            project=None,
            operator_auth_fd=None,
            non_interactive=True,
            machine=True,
            format="json",
            output=None,
        )
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(
            stderr
        ), mock.patch(
            "kassiber.operator.cli._selected_data_root",
            return_value="/canonical-project",
        ), mock.patch(
            "kassiber.operator.cli.effective_unlock_mode",
            return_value="brokered",
        ), mock.patch(
            "kassiber.cli.command_registry.command_path",
            return_value="status",
        ), mock.patch(
            "kassiber.operator.cli.BrokerClient.submit",
            return_value={"operation_id": "generation.operation", "state": "queued"},
        ), mock.patch(
            "kassiber.operator.cli.BrokerClient.wait",
            return_value=completed,
        ):
            exit_code = route_brokered_command(args, ["status"])
        return exit_code, stdout.getvalue(), stderr.getvalue()

    def test_machine_cancelled_operation_writes_typed_envelope(self) -> None:
        exit_code, stdout, _stderr = self._route_machine_command(
            {
                "operation_id": "generation.operation",
                "state": "cancelled",
                "exit_code": 1,
                "stdout": "",
                "stderr": "operator lease expired\n",
            }
        )

        self.assertEqual(exit_code, 1)
        payload = json.loads(stdout)
        self.assertEqual(payload["error"]["code"], "operator_operation_cancelled")
        self.assertEqual(payload["error"]["details"]["operation_id"], "generation.operation")
        self.assertEqual(payload["error"]["details"]["state"], "cancelled")
        self.assertTrue(payload["error"]["retryable"])

    def test_machine_unknown_result_without_child_output_writes_envelope(self) -> None:
        exit_code, stdout, stderr = self._route_machine_command(
            {
                "operation_id": "generation.operation",
                "state": "result_unknown",
                "reason": "broker_generation_changed",
            }
        )

        self.assertEqual(exit_code, 1)
        payload = json.loads(stdout)
        self.assertEqual(payload["error"]["code"], "operator_result_unknown")
        self.assertEqual(
            payload["error"]["details"]["reason"], "broker_generation_changed"
        )
        self.assertIn("operator operation status generation.operation", payload["error"]["hint"])
        self.assertIn("operator result is unknown", stderr)

    def test_machine_unknown_result_keeps_the_childs_single_envelope(self) -> None:
        child = json.dumps({"kind": "error", "schema_version": 1, "error": {"code": "conflict"}})
        exit_code, stdout, _stderr = self._route_machine_command(
            {
                "operation_id": "generation.operation",
                "state": "result_unknown",
                "exit_code": 1,
                "stdout": child + "\n",
                "stderr": "",
            }
        )

        self.assertEqual(exit_code, 1)
        self.assertEqual(json.loads(stdout)["error"]["code"], "conflict")

    def test_machine_oversized_result_writes_typed_envelope(self) -> None:
        exit_code, stdout, _stderr = self._route_machine_command(
            {
                "operation_id": "generation.operation",
                "state": "completed",
                "exit_code": 0,
                "output_available": False,
                "output_error": {
                    "code": "operator_result_too_large",
                    "message": "The operator result is too large to return.",
                },
            }
        )

        self.assertEqual(exit_code, 1)
        payload = json.loads(stdout)
        self.assertEqual(payload["error"]["code"], "operator_result_too_large")
        self.assertEqual(payload["error"]["details"]["state"], "completed")
        self.assertEqual(payload["error"]["details"]["exit_code"], 0)

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


class OperatorChildWorkingDirectoryTest(unittest.TestCase):
    def test_child_enters_the_callers_directory_before_dispatch(self) -> None:
        from kassiber.cli.main import _enter_operator_caller_directory

        previous = os.getcwd()
        with tempfile.TemporaryDirectory() as caller:
            try:
                with mock.patch.dict(
                    os.environ,
                    {
                        "KASSIBER_OPERATOR_CHILD": "1",
                        "KASSIBER_OPERATOR_WORKING_DIRECTORY": caller,
                    },
                ):
                    _enter_operator_caller_directory(mock.Mock(output="out.json"))
                self.assertEqual(os.path.realpath(os.getcwd()), os.path.realpath(caller))
            finally:
                os.chdir(previous)

    def test_directory_binding_is_refused_outside_a_broker_child(self) -> None:
        from kassiber.cli.main import _enter_operator_caller_directory

        with mock.patch.dict(
            os.environ,
            {"KASSIBER_OPERATOR_WORKING_DIRECTORY": "/", "KASSIBER_OPERATOR_CHILD": ""},
        ):
            with self.assertRaises(AppError) as raised:
                _enter_operator_caller_directory(mock.Mock(output=None))
        self.assertEqual(raised.exception.code, "operator_project_binding_invalid")

    def test_vanished_directory_fails_without_writing_relative_output(self) -> None:
        from kassiber.cli.main import _enter_operator_caller_directory

        with tempfile.TemporaryDirectory() as root:
            args = mock.Mock(output="out.json")
            with mock.patch.dict(
                os.environ,
                {
                    "KASSIBER_OPERATOR_CHILD": "1",
                    "KASSIBER_OPERATOR_WORKING_DIRECTORY": os.path.join(root, "gone"),
                },
            ):
                with self.assertRaises(AppError) as raised:
                    _enter_operator_caller_directory(args)
        self.assertEqual(raised.exception.code, "operator_working_directory_unavailable")
        self.assertIsNone(args.output)
