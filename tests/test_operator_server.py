from __future__ import annotations

import os
import signal
import stat
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from kassiber.operator.server import (
    BrokerServer,
    _error_response,
    _linux_logind_user_alive,
    _linux_session_lifetime_is_valid,
    _logind_user_state_is_alive,
    _login_session_runtime_is_valid,
    _login_session_runtime_path_is_trusted,
    _login_session_runtime_root,
    main,
)
from kassiber.errors import AppError
from kassiber.operator.protocol import PROTOCOL_VERSION, TEST_RUNTIME_OVERRIDE_ENV


class OperatorServerTest(unittest.TestCase):
    def test_native_auth_restart_stops_admission_after_response(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.prepare_idle_restart.return_value = {
            "restart": "accepted",
            "generation": "generation",
        }
        server.request_stop = mock.Mock()
        channel = mock.MagicMock()
        channel.receive_json.return_value = {
            "version": PROTOCOL_VERSION,
            "action": "restart_for_native_auth",
        }

        server._serve_channel(channel)

        server.service.prepare_idle_restart.assert_called_once_with()
        channel.send_json.assert_called_once()
        server.request_stop.assert_called_once_with()

    @unittest.skipIf(os.name == "nt", "POSIX signal handler contract")
    def test_main_signal_handler_only_requests_stop_before_final_close(self) -> None:
        server = mock.Mock()
        handlers: dict[int, object] = {}

        def register(signum: int, handler: object) -> None:
            handlers[signum] = handler

        def serve() -> None:
            handler = handlers[signal.SIGTERM]
            handler(signal.SIGTERM, None)  # type: ignore[operator]

        server.serve_forever.side_effect = serve
        with mock.patch(
            "kassiber.operator.server.BrokerServer",
            return_value=server,
        ), mock.patch(
            "kassiber.operator.server.signal.signal",
            side_effect=register,
        ):
            self.assertEqual(main(), 0)

        server.request_stop.assert_called_once_with()
        server.close.assert_called_once_with()

    def test_signal_stop_leaves_service_cleanup_for_final_close(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.listener = mock.Mock()
        server.service = mock.Mock()
        server._stopped = threading.Event()
        server._close_lock = threading.Lock()
        server._listener_closed = False

        server.request_stop()

        self.assertTrue(server._stopped.is_set())
        server.listener.close.assert_called_once_with()
        server.service.close.assert_not_called()

        server.close()

        server.listener.close.assert_called_once_with()
        server.service.close.assert_called_once_with()

    def test_reentrant_signal_stop_never_waits_on_active_close_lock(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.listener = mock.Mock()
        server.service = mock.Mock()
        server._stopped = threading.Event()
        server._close_lock = threading.Lock()
        server._listener_closed = False

        server._close_lock.acquire()
        try:
            server.request_stop()
        finally:
            server._close_lock.release()

        self.assertTrue(server._stopped.is_set())
        server.listener.close.assert_not_called()
        server.service.close.assert_not_called()

        server.close()
        server.listener.close.assert_called_once_with()
        server.service.close.assert_called_once_with()

    def test_close_retries_service_cleanup_after_listener_is_closed(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.listener = mock.Mock()
        server.service = mock.Mock()
        server.service.close.side_effect = [
            OSError("transient owner release failure"),
            None,
        ]
        server._stopped = threading.Event()
        server._close_lock = threading.Lock()
        server._listener_closed = False

        with self.assertRaisesRegex(OSError, "transient owner release failure"):
            server.close()
        server.close()

        self.assertTrue(server._stopped.is_set())
        server.listener.close.assert_called_once_with()
        self.assertEqual(server.service.close.call_count, 2)

    def test_close_still_attempts_service_when_listener_close_fails(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.listener = mock.Mock()
        server.listener.close.side_effect = [OSError("listener failure"), None]
        server.service = mock.Mock()
        server._stopped = threading.Event()
        server._close_lock = threading.Lock()
        server._listener_closed = False

        with self.assertRaisesRegex(OSError, "listener failure"):
            server.close()
        server.close()

        self.assertEqual(server.listener.close.call_count, 2)
        self.assertEqual(server.service.close.call_count, 2)

    def test_close_still_attempts_service_when_listener_is_interrupted(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.listener = mock.Mock()
        server.listener.close.side_effect = KeyboardInterrupt
        server.service = mock.Mock()
        server._stopped = threading.Event()
        server._close_lock = threading.Lock()
        server._listener_closed = False

        with self.assertRaises(KeyboardInterrupt):
            server.close()

        server.service.close.assert_called_once_with()

    def test_admin_submission_requires_challenge_bound_fresh_auth(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.submit.return_value = {
            "operation_id": "generation.operation",
            "state": "queued",
        }
        authorization = object()
        server.service.verify_admin.return_value = authorization
        channel = mock.Mock()
        authentication = bytearray(b"fresh-admin-passphrase")
        channel.receive_secret.return_value = authentication

        with mock.patch(
            "kassiber.operator.server._canonical_data_root",
            return_value="/canonical-project",
        ):
            response = server._handle_submit(
                channel,
                {
                    "data_root": "/caller-project",
                    "operation_id": "generation.operation",
                    "argv": ["secrets", "verify"],
                },
            )

        self.assertTrue(response["ok"])
        continuation = channel.send_json.call_args.args[0]
        self.assertEqual(continuation["continue"], "secrets")
        self.assertIsNotNone(continuation["admin_challenge"])
        channel.receive_secret.assert_called_once_with(
            continuation["admin_challenge"]
        )
        server.service.verify_admin.assert_called_once_with(
            "/canonical-project",
            authentication,
        )
        self.assertIs(
            server.service.submit.call_args.kwargs["admin_authorization"],
            authorization,
        )
        self.assertEqual(set(authentication), {0})

    def test_failed_admin_verification_never_submits(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.verify_admin.side_effect = AppError(
            "wrong passphrase",
            code="unlock_failed",
        )
        channel = mock.Mock()
        authentication = bytearray(b"wrong-admin-passphrase")
        channel.receive_secret.return_value = authentication

        with mock.patch(
            "kassiber.operator.server._canonical_data_root",
            return_value="/canonical-project",
        ):
            with self.assertRaises(AppError) as raised:
                server._handle_submit(
                    channel,
                    {
                        "data_root": "/caller-project",
                        "operation_id": "generation.operation",
                        "argv": ["secrets", "verify"],
                    },
                )

        self.assertEqual(raised.exception.code, "unlock_failed")
        server.service.submit.assert_not_called()
        self.assertEqual(set(authentication), {0})

    def test_multiple_secret_frames_follow_serialized_label_order(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.submit.return_value = {
            "operation_id": "generation.operation",
            "state": "queued",
        }
        channel = mock.Mock()
        first = bytearray(b"first-secret")
        second = bytearray(b"second-secret")
        channel.receive_secret.side_effect = [first, second]
        labels = ["broker-secret-z", "broker-secret-a"]

        with mock.patch(
            "kassiber.operator.server._canonical_data_root",
            return_value="/canonical-project",
        ), mock.patch(
            "kassiber.operator.server._classify_argv",
            return_value=("backends.create", mock.Mock(value="operator")),
        ):
            response = server._handle_submit(
                channel,
                {
                    "data_root": "/caller-project",
                    "operation_id": "generation.operation",
                    "argv": ["backends", "create"],
                    "secret_labels": labels,
                },
            )

        self.assertTrue(response["ok"])
        challenges = channel.send_json.call_args.args[0]["challenges"]
        self.assertEqual(
            channel.receive_secret.call_args_list,
            [
                mock.call(challenges["broker-secret-a"]),
                mock.call(challenges["broker-secret-z"]),
            ],
        )
        submitted = server.service.submit.call_args.kwargs["secret_arguments"]
        self.assertIs(submitted["broker-secret-a"], first)
        self.assertIs(submitted["broker-secret-z"], second)

    def _submit_with_caller_context(self, **fields: object) -> mock.Mock:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.submit.return_value = {
            "operation_id": "generation.operation",
            "state": "queued",
        }
        with mock.patch(
            "kassiber.operator.server._canonical_data_root",
            return_value="/canonical-project",
        ), mock.patch(
            "kassiber.operator.server._classify_argv",
            return_value=("status", mock.Mock(value="read")),
        ):
            server._handle_submit(
                mock.Mock(),
                {
                    "data_root": "/caller-project",
                    "operation_id": "generation.operation",
                    "argv": ["status"],
                    **fields,
                },
            )
        return server.service.submit

    def test_submission_binds_the_callers_directory_and_egress_switch(self) -> None:
        with tempfile.TemporaryDirectory() as caller_directory:
            submit = self._submit_with_caller_context(
                working_directory=caller_directory,
                no_egress=True,
            )
        self.assertEqual(submit.call_args.kwargs["working_directory"], caller_directory)
        self.assertIs(submit.call_args.kwargs["no_egress"], True)

    def test_older_clients_without_caller_context_keep_prior_behavior(self) -> None:
        submit = self._submit_with_caller_context()
        self.assertIsNone(submit.call_args.kwargs["working_directory"])
        self.assertIs(submit.call_args.kwargs["no_egress"], False)

    def test_submission_rejects_relative_or_missing_working_directories(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            missing = os.path.join(root, "gone")
            cases = {
                "relative": ("relative/dir", "operator_protocol_error"),
                "missing": (missing, "operator_working_directory_unavailable"),
                "not_a_string": (7, "operator_protocol_error"),
            }
            for label, (value, code) in cases.items():
                with self.subTest(label), self.assertRaises(AppError) as raised:
                    self._submit_with_caller_context(working_directory=value)
                self.assertEqual(raised.exception.code, code)

    def test_submission_rejects_a_non_boolean_egress_flag(self) -> None:
        with self.assertRaises(AppError) as raised:
            self._submit_with_caller_context(no_egress="1")
        self.assertEqual(raised.exception.code, "operator_protocol_error")

    def test_an_appimage_has_one_identity_across_launches(self) -> None:
        from kassiber.operator import build

        identities = []
        with tempfile.TemporaryDirectory() as tmp:
            for mount in ("mount_a", "mount_b"):
                sidecar = Path(tmp) / mount / "usr" / "bin" / "kassiber-sidecar"
                sidecar.parent.mkdir(parents=True)
                sidecar.touch()
                with mock.patch.object(build.sys, "frozen", True, create=True), mock.patch.object(
                    build.sys, "executable", str(sidecar)
                ), mock.patch.dict(
                    os.environ,
                    {"APPIMAGE": str(Path(tmp) / "Kassiber.AppImage"), "APPDIR": str(Path(tmp) / mount)},
                ):
                    identities.append(build.build_identity())
            # An executable outside the mount does not borrow the AppImage's.
            outside = Path(tmp) / "other" / "kassiber"
            outside.parent.mkdir()
            outside.touch()
            with mock.patch.object(build.sys, "frozen", True, create=True), mock.patch.object(
                build.sys, "executable", str(outside)
            ), mock.patch.dict(
                os.environ,
                {"APPIMAGE": str(Path(tmp) / "Kassiber.AppImage"), "APPDIR": str(Path(tmp) / "mount_a")},
            ):
                stranger = build.build_identity()
        self.assertEqual(identities[0], identities[1])
        self.assertNotEqual(stranger["origin"], identities[0]["origin"])

    def test_ping_and_status_name_the_brokers_build(self) -> None:
        from kassiber.operator.build import build_identity

        server = BrokerServer.__new__(BrokerServer)
        server.generation = "generation"
        server.service = mock.Mock()
        server.service.status.return_value = {"broker": "running", "lease": "locked"}
        with mock.patch(
            "kassiber.operator.native_auth.native_auth_helper_identity",
            return_value=None,
        ):
            ping = server._handle(mock.Mock(), {"action": "ping"})
        self.assertEqual(ping["data"]["build"], build_identity())
        status = server._handle(mock.Mock(), {"action": "status"})
        self.assertEqual(status["data"]["broker_build"], build_identity())

    def test_the_command_line_follows_only_after_the_broker_names_its_build(self) -> None:
        from kassiber.operator.build import build_identity

        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.submit.return_value = {"operation_id": "g.client.1", "state": "queued"}
        channel = mock.MagicMock()
        channel.peer_pid = 7
        channel.receive_json.return_value = {"argv": ["--data-root", "/project", "--machine", "status"]}
        with mock.patch("kassiber.operator.server._canonical_data_root", return_value="/project"):
            response = server._handle(
                channel,
                {"action": "submit", "data_root": "/project", "operation_id": "g.client.1", "argv_follows": True},
            )
        continuation = channel.send_json.call_args_list[0].args[0]
        self.assertEqual(continuation, {"ok": True, "continue": "argv", "build": build_identity()})
        self.assertEqual(server.service.submit.call_args.args[1][-1], "status")
        self.assertEqual(response["data"]["build"], build_identity())

    def test_submit_for_another_build_is_refused_before_admission(self) -> None:
        from kassiber.operator.build import build_identity

        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        channel = mock.Mock()
        with self.assertRaises(AppError) as raised:
            server._handle(
                channel,
                {
                    "action": "submit",
                    "data_root": "/project",
                    "operation_id": "g.client.1",
                    "argv": ["--machine", "status"],
                    "expected_build": {**build_identity(), "origin": "f" * 12},
                },
            )
        self.assertEqual(raised.exception.code, "operator_broker_build_mismatch")
        server.service.submit.assert_not_called()
        channel.send_json.assert_not_called()

    def test_password_unlock_cannot_claim_touch_id_authentication(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        with self.assertRaises(AppError) as raised:
            server._handle(
                mock.Mock(),
                {
                    "action": "unlock",
                    "data_root": "/public-placeholder",
                    "authentication_method": "touch_id",
                },
            )
        self.assertEqual(
            raised.exception.code,
            "operator_invalid_authentication_method",
        )
        server.service.unlock.assert_not_called()

    def test_touch_id_method_is_assigned_by_the_broker(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.unlock.return_value = {"lease": "unlocked"}
        secret = bytearray(b"native-secret")
        with mock.patch(
            "kassiber.operator.native_auth.broker_touch_id_passphrase",
            return_value=secret,
        ), mock.patch(
            "kassiber.operator.native_auth.validate_native_auth_helper_identity",
        ), mock.patch(
            "kassiber.operator.server._canonical_data_root",
            return_value="/canonical-project",
        ), mock.patch(
            "kassiber.operator.server.require_project_policy_binding",
            return_value=mock.Mock(
                project_identity="p" * 64,
                database_identity="d" * 32,
            ),
        ):
            response = server._handle(
                mock.Mock(),
                {
                    "action": "unlock_touch_id",
                    "data_root": "/public-placeholder",
                    "capability": "operator",
                    "native_auth_identity": "signed-helper",
                },
            )
        self.assertTrue(response["ok"])
        self.assertEqual(
            server.service.unlock.call_args.kwargs["authentication_method"],
            "touch_id",
        )
        self.assertEqual(
            server.service.unlock.call_args.kwargs["expected_project_identity"],
            "p" * 64,
        )
        self.assertEqual(
            server.service.unlock.call_args.kwargs["expected_database_identity"],
            "d" * 32,
        )
        self.assertEqual(set(secret), {0})

    def test_touch_id_configuration_is_challenge_bound_and_wipes_auth(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        server.service.configure_touch_id_authenticated.return_value = {
            "configured": True,
            "auth": "touch_id",
        }
        channel = mock.Mock()
        authentication = bytearray(b"fresh-passphrase")
        channel.receive_secret.return_value = authentication

        with mock.patch(
            "kassiber.operator.native_auth.validate_native_auth_helper_identity",
        ):
            response = server._handle(
                channel,
                {
                    "action": "touch_id_configure",
                    "data_root": "/public-placeholder",
                    "configured": True,
                    "native_auth_identity": "signed-helper",
                },
            )

        self.assertTrue(response["ok"])
        challenge = channel.send_json.call_args.args[0]["challenge"]
        channel.receive_secret.assert_called_once_with(challenge)
        server.service.configure_touch_id_authenticated.assert_called_once_with(
            str(Path("/public-placeholder").resolve()),
            authentication,
            configured=True,
            native_auth_identity="signed-helper",
        )
        self.assertEqual(set(authentication), {0})

    def test_touch_id_configuration_rejects_non_boolean_state(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        with mock.patch(
            "kassiber.operator.native_auth.validate_native_auth_helper_identity",
        ), self.assertRaises(AppError) as raised:
            server._handle(
                mock.Mock(),
                {
                    "action": "touch_id_configure",
                    "data_root": "/public-placeholder",
                    "configured": "yes",
                    "native_auth_identity": "signed-helper",
                },
            )
        self.assertEqual(raised.exception.code, "operator_protocol_error")
        server.service.configure_touch_id_authenticated.assert_not_called()

    def test_touch_id_configuration_rejects_helper_mismatch_before_challenge(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        channel = mock.Mock()
        with mock.patch(
            "kassiber.operator.native_auth.validate_native_auth_helper_identity",
            side_effect=AppError(
                "helper mismatch",
                code="native_auth_helper_mismatch",
            ),
        ), self.assertRaises(AppError) as raised:
            server._handle(
                channel,
                {
                    "action": "touch_id_configure",
                    "data_root": "/public-placeholder",
                    "configured": True,
                    "native_auth_identity": "signed-helper",
                },
            )

        self.assertEqual(raised.exception.code, "native_auth_helper_mismatch")
        channel.send_json.assert_not_called()
        channel.receive_secret.assert_not_called()
        server.service.configure_touch_id_authenticated.assert_not_called()

    def test_error_details_recursively_redact_secret_keys_and_values(self) -> None:
        response = _error_response(
            AppError(
                "safe",
                code="test",
                details={
                    "token": "token-value",
                    "nested": {
                        "blinding-key": "blind-value",
                        "message": "passphrase=hunter2",
                    },
                },
            )
        )
        rendered = repr(response)
        self.assertNotIn("token-value", rendered)
        self.assertNotIn("blind-value", rendered)
        self.assertNotIn("hunter2", rendered)

    @unittest.skipUnless(os.name == "posix", "POSIX ownership test")
    def test_linux_session_runtime_disappearance_invalidates_session(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.server.sys.platform", "linux"
        ), mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": tmp}), mock.patch(
            "kassiber.operator.server._login_session_runtime_path_is_trusted",
            return_value=True,
        ):
            runtime = _login_session_runtime_root()
            self.assertEqual(runtime.root, Path(tmp).resolve())
            self.assertTrue(_login_session_runtime_is_valid(runtime))
        self.assertFalse(_login_session_runtime_is_valid(runtime))

    @unittest.skipUnless(os.name == "posix", "POSIX runtime identity test")
    def test_linux_session_runtime_replacement_invalidates_session(self) -> None:
        with tempfile.TemporaryDirectory() as parent, mock.patch(
            "kassiber.operator.server.sys.platform", "linux"
        ):
            root = Path(parent) / "runtime"
            root.mkdir(mode=0o700)
            with mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": str(root)}), mock.patch(
                "kassiber.operator.server._login_session_runtime_path_is_trusted",
                return_value=True,
            ):
                runtime = _login_session_runtime_root()
                root.rename(Path(parent) / "displaced-runtime")
                root.mkdir(mode=0o700)
                self.assertFalse(_login_session_runtime_is_valid(runtime))

    def test_linux_refuses_broker_without_logout_lifetime_primitive(self) -> None:
        with mock.patch(
            "kassiber.operator.server.sys.platform", "linux"
        ), mock.patch(
            "kassiber.operator.server._login_session_runtime_root",
            return_value=None,
        ), mock.patch(
            "kassiber.operator.server._linux_logind_user_alive",
            return_value=None,
        ), mock.patch("kassiber.operator.server.listen") as listen:
            with self.assertRaises(AppError) as raised:
                BrokerServer()
        self.assertEqual(
            raised.exception.code,
            "operator_session_lifetime_unavailable",
        )
        listen.assert_not_called()

    @unittest.skipUnless(os.name == "posix", "POSIX runtime trust test")
    def test_linux_rejects_caller_selected_persistent_runtime_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.server.sys.platform", "linux"
        ), mock.patch.dict(
            os.environ,
            {"XDG_RUNTIME_DIR": tmp, TEST_RUNTIME_OVERRIDE_ENV: "1"},
        ), mock.patch(
            "kassiber.operator.server._linux_logind_user_alive",
            return_value=None,
        ), mock.patch("kassiber.operator.server.listen") as listen:
            self.assertIsNone(_login_session_runtime_root())
            with self.assertRaises(AppError) as raised:
                BrokerServer()
        self.assertEqual(raised.exception.code, "operator_session_lifetime_unavailable")
        listen.assert_not_called()

    @unittest.skipUnless(os.name == "posix", "POSIX runtime trust test")
    def test_linux_runtime_fallback_requires_canonical_volatile_owner_directory(
        self,
    ) -> None:
        info = mock.Mock(st_mode=stat.S_IFDIR | 0o700, st_uid=1000)
        with mock.patch("kassiber.operator.server.os.getuid", return_value=1000), mock.patch(
            "kassiber.operator.server._linux_runtime_filesystem_is_volatile",
            return_value=True,
        ):
            self.assertTrue(
                _login_session_runtime_path_is_trusted(Path("/run/user/1000"), info)
            )
            self.assertFalse(
                _login_session_runtime_path_is_trusted(Path("/tmp/persistent"), info)
            )
            info.st_mode = stat.S_IFDIR | 0o755
            self.assertFalse(
                _login_session_runtime_path_is_trusted(Path("/run/user/1000"), info)
            )

    def test_linux_logind_guard_fails_closed_after_query_loss(self) -> None:
        self.assertFalse(
            _linux_session_lifetime_is_valid(
                None,
                logind_observed=True,
                logind_alive=None,
            )
        )

    def test_linux_runtime_guard_can_cover_transient_logind_query_loss(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.server.sys.platform", "linux"
        ), mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": tmp}), mock.patch(
            "kassiber.operator.server._login_session_runtime_path_is_trusted",
            return_value=True,
        ):
            runtime = _login_session_runtime_root()
            self.assertTrue(
                _linux_session_lifetime_is_valid(
                    runtime,
                    logind_observed=True,
                    logind_alive=None,
                )
            )

    def test_non_linux_has_no_runtime_session_guard(self) -> None:
        with mock.patch("kassiber.operator.server.sys.platform", "darwin"):
            self.assertIsNone(_login_session_runtime_root())

    def test_non_linux_has_no_logind_user_guard(self) -> None:
        with mock.patch("kassiber.operator.server.sys.platform", "darwin"):
            self.assertIsNone(_linux_logind_user_alive())

    def test_missing_logind_user_record_is_not_misreported_as_logout(self) -> None:
        systemd = mock.Mock()
        systemd.sd_uid_get_state.return_value = -1
        with mock.patch(
            "kassiber.operator.server._load_systemd", return_value=systemd
        ):
            self.assertIsNone(_linux_logind_user_alive())

    def test_logind_user_state_distinguishes_login_from_linger(self) -> None:
        self.assertTrue(_logind_user_state_is_alive("online"))
        self.assertTrue(_logind_user_state_is_alive("active"))
        self.assertFalse(_logind_user_state_is_alive("closing"))
        self.assertFalse(_logind_user_state_is_alive("lingering"))
        self.assertFalse(_logind_user_state_is_alive("offline"))


if __name__ == "__main__":
    unittest.main()
