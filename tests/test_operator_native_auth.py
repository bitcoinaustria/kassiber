from __future__ import annotations

import tempfile
import unittest
import io
import os
from unittest import mock
from pathlib import Path

from kassiber.operator.native_auth import (
    broker_touch_id_passphrase,
    invalidate_operator_native_auth,
    operator_touch_id_account,
    touch_id_status,
)


class OperatorNativeAuthTest(unittest.TestCase):
    def test_rotation_generation_changes_opaque_native_account(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
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
            self.assertEqual(
                touch_id_status(tmp),
                {
                    "available": False,
                    "configured": False,
                    "reason": "native_auth_unavailable",
                },
            )

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
            "kassiber.operator.native_auth.subprocess.Popen",
            side_effect=spawn,
        ) as popen:
            secret = broker_touch_id_passphrase(tmp)
            try:
                self.assertEqual(bytes(secret), b"touch-id-secret")
            finally:
                secret[:] = b"\0" * len(secret)
            command = popen.call_args.args[0]
            self.assertIn("broker-get", command)
            self.assertIn("--output-fd", command)
            self.assertNotIn(tmp, command)

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
