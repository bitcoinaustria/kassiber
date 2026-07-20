from __future__ import annotations

import os
import socket
import struct
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from kassiber.errors import AppError
from kassiber.operator.protocol import (
    BrokerChannel,
    _SocketTransport,
    _verify_unix_peer,
    connect,
    listen,
    operator_runtime_dir,
)
from kassiber.operator.client import BrokerClient


class OperatorProtocolTest(unittest.TestCase):
    def test_json_and_challenge_bound_secret_frames_are_distinct(self) -> None:
        left, right = socket.socketpair()
        sender = BrokerChannel(_SocketTransport(left))
        receiver = BrokerChannel(_SocketTransport(right))
        try:
            sender.send_json({"action": "unlock", "version": 1})
            self.assertEqual(
                receiver.receive_json(),
                {"action": "unlock", "version": 1},
            )
            sender.send_secret("challenge", "not-in-json")
            secret = receiver.receive_secret("challenge")
            self.assertEqual(bytes(secret), b"not-in-json")
        finally:
            sender.close()
            receiver.close()

    def test_wrong_secret_challenge_fails_closed(self) -> None:
        left, right = socket.socketpair()
        sender = BrokerChannel(_SocketTransport(left))
        receiver = BrokerChannel(_SocketTransport(right))
        try:
            sender.send_secret("first", "secret")
            with self.assertRaisesRegex(AppError, "challenge") as raised:
                receiver.receive_secret("second")
            self.assertEqual(raised.exception.code, "operator_secret_challenge_mismatch")
        finally:
            sender.close()
            receiver.close()

    @unittest.skipIf(os.name == "nt", "Unix socket test")
    def test_listener_authenticates_same_user_and_round_trips(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": tmp}
        ):
            os.chmod(tmp, 0o700)
            server = listen()
            result: list[dict] = []

            def serve() -> None:
                with server.accept() as channel:
                    result.append(channel.receive_json())
                    channel.send_json({"ok": True})

            thread = threading.Thread(target=serve)
            thread.start()
            with connect() as channel:
                channel.send_json({"ping": True})
                self.assertEqual(channel.receive_json(), {"ok": True})
            thread.join(2)
            server.close()
            self.assertEqual(result, [{"ping": True}])

    @unittest.skipIf(os.name == "nt", "Unix permission test")
    def test_permissive_runtime_directory_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": tmp}
        ):
            os.chmod(tmp, 0o755)
            with self.assertRaises(AppError) as raised:
                operator_runtime_dir()
            self.assertEqual(raised.exception.code, "unsafe_operator_runtime_directory")

    @unittest.skipIf(os.name == "nt", "Unix socket test")
    def test_owned_stale_socket_is_recovered(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": tmp}
        ):
            os.chmod(tmp, 0o700)
            endpoint = Path(tmp) / "operator-v1.sock"
            stale = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            stale.bind(str(endpoint))
            stale.close()
            server = listen()
            try:
                self.assertTrue(endpoint.exists())
            finally:
                server.close()

    @unittest.skipIf(os.name == "nt", "Unix startup lock test")
    def test_startup_lock_prevents_a_second_listener_from_touching_endpoint(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": tmp}
        ):
            os.chmod(tmp, 0o700)
            server = listen()
            try:
                with self.assertRaises(AppError) as raised:
                    listen()
                self.assertEqual(raised.exception.code, "operator_broker_running")
                self.assertTrue((Path(tmp) / "operator-v1.sock").exists())
            finally:
                server.close()

    @unittest.skipIf(os.name == "nt", "Unix inode test")
    def test_listener_close_does_not_unlink_a_replacement_endpoint(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": tmp}
        ):
            os.chmod(tmp, 0o700)
            server = listen()
            endpoint = Path(tmp) / "operator-v1.sock"
            displaced = Path(tmp) / "displaced.sock"
            endpoint.rename(displaced)
            replacement = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            replacement.bind(str(endpoint))
            try:
                server.close()
                self.assertTrue(endpoint.exists())
            finally:
                replacement.close()
                endpoint.unlink(missing_ok=True)
                displaced.unlink(missing_ok=True)

    @unittest.skipUnless(sys.platform.startswith("linux"), "SO_PEERCRED test")
    def test_cross_user_unix_peer_is_rejected(self) -> None:
        peer = mock.Mock()
        peer.getsockopt.return_value = struct.pack("3i", 1234, os.getuid() + 1, 1234)
        with self.assertRaises(AppError) as raised:
            _verify_unix_peer(peer)
        self.assertEqual(raised.exception.code, "operator_peer_rejected")

    def test_windows_contract_names_acl_and_bilateral_sid_checks(self) -> None:
        source = Path(__file__).parents[1] / "kassiber" / "operator" / "protocol.py"
        text = source.read_text(encoding="utf-8")
        for primitive in (
            "PIPE_REJECT_REMOTE_CLIENTS",
            "FILE_FLAG_FIRST_PIPE_INSTANCE",
            "ConvertStringSecurityDescriptorToSecurityDescriptorW",
            "GetNamedPipeClientProcessId",
            "GetNamedPipeServerProcessId",
            "GetNamedSecurityInfoW",
            "PeekNamedPipe",
            "operator named-pipe read timed out",
            "DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS",
        ):
            with self.subTest(primitive=primitive):
                self.assertIn(primitive, text)
        listener_accept = text.split("class _WindowsBrokerListener:", 1)[1]
        listener_accept = listener_accept.split("        def close(self)", 1)[0]
        self.assertIn(
            "io_timeout=DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS",
            listener_accept,
        )

    @unittest.skipIf(os.name == "nt", "Unix timeout test")
    def test_ping_read_is_bounded_when_endpoint_accepts_but_never_replies(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": tmp}
        ):
            os.chmod(tmp, 0o700)
            endpoint = Path(tmp) / "operator-v1.sock"
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(endpoint))
            listener.listen(1)
            release = threading.Event()

            def wedge() -> None:
                connection, _ = listener.accept()
                try:
                    release.wait(2)
                finally:
                    connection.close()

            thread = threading.Thread(target=wedge)
            thread.start()
            started = __import__("time").monotonic()
            try:
                with self.assertRaises(TimeoutError):
                    BrokerClient().ping()
                self.assertLess(__import__("time").monotonic() - started, 1.5)
            finally:
                release.set()
                listener.close()
                thread.join(2)


if __name__ == "__main__":
    unittest.main()
