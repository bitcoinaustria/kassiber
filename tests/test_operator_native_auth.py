from __future__ import annotations

import tempfile
import unittest
import io
import os
import sys
from unittest import mock
from pathlib import Path

from kassiber.errors import AppError
from kassiber.operator.native_auth import (
    _MACOS_APP_EXECUTABLE_NAME,
    _helper_path,
    _signed_helper_code,
    _spawn_validated_helper,
    broker_touch_id_passphrase,
    invalidate_operator_native_auth,
    native_auth_helper_identity,
    operator_touch_id_account,
    touch_id_store,
    touch_id_status,
)
from kassiber.operator.policy import bind_project_policy


TEST_DATABASE_IDENTITY = "d" * 32


class OperatorNativeAuthTest(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "macOS helper launch uses inherited POSIX fds")
    def test_launch_gate_keeps_helper_alive_through_pid_validation(self) -> None:
        script = (
            "import os,sys;"
            "ready=int(sys.argv[sys.argv.index('--ready-fd')+1]);"
            "go=int(sys.argv[sys.argv.index('--go-fd')+1]);"
            "os.write(ready,b'R');os.close(ready);"
            "signal=os.read(go,1);os.close(go);"
            "raise SystemExit(0 if signal==b'G' else 2)"
        )

        def validate(process, expected_identity):
            self.assertIsNone(process.poll())
            self.assertEqual(expected_identity, "signed-helper")

        with mock.patch(
            "kassiber.operator.native_auth._validate_spawned_helper",
            side_effect=validate,
        ):
            process = _spawn_validated_helper(
                [sys.executable, "-c", script],
                inherited_fds=(),
                expected_identity="signed-helper",
            )

        try:
            self.assertEqual(process.wait(timeout=2), 0)
        finally:
            if process.stderr is not None:
                process.stderr.close()

    def test_helper_identity_detects_replacement_before_secret_handoff(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            helper = (
                Path(tmp)
                / "Kassiber.app"
                / "Contents"
                / "MacOS"
                / _MACOS_APP_EXECUTABLE_NAME
            )
            helper.parent.mkdir(parents=True)
            helper.write_bytes(b"signed-helper")
            helper.chmod(0o700)
            with mock.patch(
                "kassiber.operator.native_auth.sys.platform",
                "darwin",
            ), mock.patch.dict(
                os.environ,
                {"KASSIBER_NATIVE_AUTH_HELPER": str(helper)},
            ), mock.patch(
                "kassiber.operator.native_auth._signed_helper_identity",
                side_effect=["signed-helper", "capture-helper"],
            ):
                expected = native_auth_helper_identity()
                helper.write_bytes(b"capture-helper")

                with self.assertRaises(AppError) as raised:
                    _helper_path(expected)

        self.assertEqual(raised.exception.code, "native_auth_helper_mismatch")

    def test_signed_helper_path_matches_packaged_macos_executable(self) -> None:
        helper = (
            Path("/Applications")
            / "Kassiber.app"
            / "Contents"
            / "MacOS"
            / _MACOS_APP_EXECUTABLE_NAME
        )
        identity = mock.Mock()
        with mock.patch(
            "kassiber.operator.native_auth._inspect_code",
            return_value=identity,
        ) as inspect:
            self.assertIs(_signed_helper_code(helper), identity)
            with self.assertRaises(AppError) as raised:
                _signed_helper_code(helper.with_name("Kassiber"))

        self.assertEqual(raised.exception.code, "native_auth_unavailable")
        inspect.assert_called_once_with(str(helper))

    def test_rotation_generation_changes_opaque_native_account(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            bind_project_policy(tmp, TEST_DATABASE_IDENTITY)
            before = operator_touch_id_account(tmp)
            generation = invalidate_operator_native_auth(tmp)
            after = operator_touch_id_account(tmp)
            self.assertNotEqual(before, after)
            self.assertEqual(len(before), 64)
            self.assertEqual(len(after), 64)
            self.assertNotIn(tmp, before)
            self.assertEqual(len(generation), 32)

    def test_touch_id_status_is_truthful_without_signed_macos_helper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            bind_project_policy(tmp, TEST_DATABASE_IDENTITY)
            self.assertEqual(
                touch_id_status(tmp),
                {
                    "available": False,
                    "configured": False,
                    "reason": "native_auth_unavailable",
                },
            )

    def test_touch_id_status_uses_the_live_validated_helper_gate(self) -> None:
        process = mock.Mock()
        process.stderr = io.BytesIO(b"")
        process.wait.return_value = 4
        process.poll.return_value = 4
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.native_auth._helper_path",
            return_value=Path("/signed/helper"),
        ), mock.patch(
            "kassiber.operator.native_auth._spawn_validated_helper",
            return_value=process,
        ) as spawn_helper:
            bind_project_policy(tmp, TEST_DATABASE_IDENTITY)
            status = touch_id_status(tmp)

        self.assertEqual(status, {"available": True, "configured": False})
        self.assertIn("status", spawn_helper.call_args.args[0])

    def test_broker_touch_id_secret_uses_an_inherited_pipe(self) -> None:
        process = mock.Mock()
        process.stderr = io.BytesIO(b"")
        process.wait.return_value = 0
        process.poll.return_value = 0

        def spawn(command, **_kwargs):
            output_fd = int(command[command.index("--output-fd") + 1])
            os.write(output_fd, b"touch-id-secret")
            return process

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.native_auth._helper_path",
            return_value=Path("/signed/helper"),
        ), mock.patch(
            "kassiber.operator.native_auth._spawn_validated_helper",
            side_effect=spawn,
        ) as spawn_helper:
            bind_project_policy(tmp, TEST_DATABASE_IDENTITY)
            secret = broker_touch_id_passphrase(tmp)
            try:
                self.assertEqual(bytes(secret), b"touch-id-secret")
            finally:
                secret[:] = b"\0" * len(secret)
            command = spawn_helper.call_args.args[0]
            self.assertIn("broker-get", command)
            self.assertIn("--output-fd", command)
            self.assertNotIn(tmp, command)

    def test_store_validates_live_helper_before_writing_passphrase(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.native_auth._helper_path",
            return_value=Path("/signed/helper"),
        ), mock.patch(
            "kassiber.operator.native_auth._spawn_validated_helper",
            side_effect=AppError(
                "invalid live helper",
                code="native_auth_helper_mismatch",
            ),
        ) as spawn_helper, mock.patch(
            "kassiber.operator.native_auth._write_fd",
        ) as write_secret:
            bind_project_policy(tmp, TEST_DATABASE_IDENTITY)
            with self.assertRaises(AppError) as raised:
                touch_id_store(
                    tmp,
                    bytearray(b"fresh-passphrase"),
                    expected_helper_identity="signed-helper",
                )

        self.assertEqual(raised.exception.code, "native_auth_helper_mismatch")
        self.assertEqual(
            spawn_helper.call_args.kwargs["expected_identity"],
            "signed-helper",
        )
        write_secret.assert_not_called()

    def test_broker_touch_id_rejects_unbound_project_before_helper_lookup(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.native_auth.subprocess.Popen",
        ) as popen, self.assertRaises(AppError) as raised:
            broker_touch_id_passphrase(tmp)

        self.assertEqual(raised.exception.code, "operator_policy_binding_required")
        popen.assert_not_called()

    def test_signed_helper_requires_broker_parent_for_all_actions(self) -> None:
        source = (
            Path(__file__).parents[1]
            / "ui-tauri"
            / "src-tauri"
            / "src"
            / "lib.rs"
        ).read_text(encoding="utf-8")
        helper = source.split("fn run_operator_native_auth_helper()", 1)[1]
        helper = helper.split("#[cfg(not(target_os = \"macos\"))]", 1)[0]
        self.assertNotIn('"unlock" =>', helper)
        self.assertIn('"broker-get" =>', helper)
        self.assertIn('"--output-fd"', helper)
        self.assertIn("verify_operator_helper_parent()?", helper)
        self.assertIn("wait_for_operator_broker_release(&args)?", helper)
        self.assertIn('helper_fd(args, "--ready-fd")?', source)
        self.assertIn('helper_fd(args, "--go-fd")?', source)
        self.assertIn('Command::new("/usr/bin/codesign")', source)
        self.assertIn("TeamIdentifier=", source)
        self.assertIn("operator_sidecar_filename_for_arch", source)
        self.assertIn("parent_name != expected_parent_name", source)
        self.assertIn("parent_identifier != expected_parent_name", source)
        self.assertIn("1.2.840.113635.100.6.1.13", source)
        self.assertIn("SecCodeCopyGuestWithAttributes", source)
        self.assertIn("SecCodeCheckValidity", source)
        self.assertNotIn("verified_codesign_requirement", source)


if __name__ == "__main__":
    unittest.main()
