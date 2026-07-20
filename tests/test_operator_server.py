from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from kassiber.operator.server import (
    BrokerServer,
    _error_response,
    _linux_logind_session_active,
    _login_session_runtime_is_valid,
    _login_session_runtime_root,
)
from kassiber.errors import AppError


class OperatorServerTest(unittest.TestCase):
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
        ):
            response = server._handle(
                mock.Mock(),
                {
                    "action": "unlock_touch_id",
                    "data_root": "/public-placeholder",
                    "capability": "operator",
                },
            )
        self.assertTrue(response["ok"])
        self.assertEqual(
            server.service.unlock.call_args.kwargs["authentication_method"],
            "touch_id",
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

        response = server._handle(
            channel,
            {
                "action": "touch_id_configure",
                "data_root": "/public-placeholder",
                "configured": True,
            },
        )

        self.assertTrue(response["ok"])
        challenge = channel.send_json.call_args.args[0]["challenge"]
        channel.receive_secret.assert_called_once_with(challenge)
        server.service.configure_touch_id_authenticated.assert_called_once_with(
            "/public-placeholder",
            authentication,
            configured=True,
        )
        self.assertEqual(set(authentication), {0})

    def test_touch_id_configuration_rejects_non_boolean_state(self) -> None:
        server = BrokerServer.__new__(BrokerServer)
        server.service = mock.Mock()
        with self.assertRaises(AppError) as raised:
            server._handle(
                mock.Mock(),
                {
                    "action": "touch_id_configure",
                    "data_root": "/public-placeholder",
                    "configured": "yes",
                },
            )
        self.assertEqual(raised.exception.code, "operator_protocol_error")
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
        ), mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": tmp}):
            root = _login_session_runtime_root()
            self.assertEqual(root, Path(tmp).resolve())
            self.assertTrue(_login_session_runtime_is_valid(root))
        self.assertFalse(_login_session_runtime_is_valid(root))

    def test_non_linux_has_no_runtime_session_guard(self) -> None:
        with mock.patch("kassiber.operator.server.sys.platform", "darwin"):
            self.assertIsNone(_login_session_runtime_root())

    def test_missing_logind_identity_is_not_misreported_as_inactive(self) -> None:
        self.assertIsNone(_linux_logind_session_active(None))


if __name__ == "__main__":
    unittest.main()
