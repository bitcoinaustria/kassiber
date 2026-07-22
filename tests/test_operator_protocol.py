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
from kassiber.operator import protocol as operator_protocol
from kassiber.operator.protocol import (
    MAX_SECRET_PAYLOAD_BYTES,
    SECRET_CHALLENGE_WIRE_BYTES,
    BrokerChannel,
    _SocketTransport,
    _verify_unix_peer,
    connect,
    listen,
    operator_runtime_dir,
)
from kassiber.operator.client import BrokerClient


class OperatorProtocolTest(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt", "Windows path-owner contract")
    def test_windows_accepts_a_temp_path_owned_by_the_effective_account(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(
                operator_protocol.windows_path_owned_by_current_user(tmp)
            )

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

    def test_secret_payload_limit_round_trips_at_the_shared_boundary(self) -> None:
        class LoopbackTransport:
            def __init__(self) -> None:
                self.buffer = bytearray()

            def send_all(self, payload: bytes) -> None:
                self.buffer.extend(payload)

            def recv_exact(self, size: int) -> bytes:
                if len(self.buffer) < size:
                    raise EOFError
                payload = bytes(self.buffer[:size])
                del self.buffer[:size]
                return payload

            def close(self) -> None:
                pass

        transport = LoopbackTransport()
        sender = BrokerChannel(transport)
        receiver = BrokerChannel(transport)
        challenge = "a" * SECRET_CHALLENGE_WIRE_BYTES
        payload = b"s" * MAX_SECRET_PAYLOAD_BYTES
        try:
            sender.send_secret(challenge, payload)
            secret = receiver.receive_secret(challenge)
            self.assertEqual(bytes(secret), payload)
            with self.assertRaises(AppError) as raised:
                sender.send_secret(challenge, payload + b"s")
            self.assertEqual(raised.exception.code, "operator_secret_too_large")
        finally:
            sender.close()
            receiver.close()

    @unittest.skipIf(os.name == "nt", "Unix socket test")
    def test_listener_authenticates_same_user_and_round_trips(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": tmp,
                operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
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
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": tmp,
                operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
        ):
            os.chmod(tmp, 0o755)
            with self.assertRaises(AppError) as raised:
                operator_runtime_dir()
            self.assertEqual(raised.exception.code, "unsafe_operator_runtime_directory")

    @unittest.skipIf(os.name == "nt", "Unix socket test")
    def test_owned_stale_socket_is_recovered(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": tmp,
                operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
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
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": tmp,
                operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
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
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": tmp,
                operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
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

    @unittest.skipIf(os.name == "nt", "Unix stable endpoint test")
    def test_production_environment_variation_cannot_split_broker_election(self) -> None:
        # macOS places its default temporary directory below a long
        # /var/folders path while AF_UNIX endpoints are limited to 104 bytes.
        # Keep this election test focused on environment independence; socket
        # path-length handling is not what it is asserting.
        with tempfile.TemporaryDirectory(prefix="kb-op-", dir="/tmp") as tmp:
            root = Path(tmp)
            account = root / "account"
            account.mkdir(mode=0o700)
            environments = [
                {
                    "HOME": str(root / "caller-one"),
                    "XDG_RUNTIME_DIR": str(root / "runtime-one"),
                    "KASSIBER_OPERATOR_RUNTIME_DIR": str(root / "override-one"),
                    operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "0",
                },
                {
                    "HOME": str(root / "caller-two"),
                    "XDG_RUNTIME_DIR": str(root / "runtime-two"),
                    "KASSIBER_OPERATOR_RUNTIME_DIR": str(root / "override-two"),
                    operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "0",
                },
            ]
            with mock.patch(
                "pwd.getpwuid",
                return_value=mock.Mock(pw_dir=str(account)),
            ), mock.patch.dict(os.environ, environments[0]):
                server = listen()
                try:
                    with mock.patch.dict(os.environ, environments[1]):
                        with self.assertRaises(AppError) as raised:
                            listen()
                        self.assertEqual(
                            raised.exception.code,
                            "operator_broker_running",
                        )
                        result: list[dict] = []

                        def serve() -> None:
                            with server.accept() as channel:
                                result.append(channel.receive_json())
                                channel.send_json({"winner": True})

                        thread = threading.Thread(target=serve)
                        thread.start()
                        with connect() as channel:
                            channel.send_json({"same_account": True})
                            self.assertEqual(
                                channel.receive_json(),
                                {"winner": True},
                            )
                        thread.join(2)
                        self.assertFalse(thread.is_alive())
                        self.assertEqual(result, [{"same_account": True}])
                finally:
                    server.close()

    @unittest.skipIf(os.name == "nt", "Unix test override gate")
    def test_runtime_override_requires_explicit_nonfrozen_test_gate(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            tempfile.TemporaryDirectory() as override,
        ):
            account = Path(tmp) / "account"
            account.mkdir(mode=0o700)
            account_runtime = account / ".kassiber" / "run"
            base_environment = {
                "HOME": str(Path(tmp) / "caller-home"),
                "XDG_RUNTIME_DIR": str(Path(tmp) / "caller-runtime"),
                "KASSIBER_OPERATOR_RUNTIME_DIR": override,
            }
            with mock.patch(
                "pwd.getpwuid",
                return_value=mock.Mock(pw_dir=str(account)),
            ), mock.patch.object(sys, "frozen", False, create=True):
                with mock.patch.dict(
                    os.environ,
                    {
                        **base_environment,
                        operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "0",
                    },
                ):
                    self.assertEqual(
                        operator_runtime_dir(),
                        account_runtime.resolve(),
                    )
                with mock.patch.dict(
                    os.environ,
                    {
                        **base_environment,
                        operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
                    },
                ):
                    self.assertEqual(
                        operator_runtime_dir(),
                        Path(override).resolve(),
                    )
                with mock.patch.object(sys, "frozen", True), mock.patch.dict(
                    os.environ,
                    {
                        **base_environment,
                        operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
                    },
                ):
                    self.assertEqual(
                        operator_runtime_dir(),
                        account_runtime.resolve(),
                    )

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
            "operator named-pipe read timed out",
            "operator named-pipe write timed out",
            "FILE_FLAG_OVERLAPPED",
            "GetOverlappedResultEx",
            "CancelIoEx",
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

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe timeout test")
    def test_windows_accepted_client_partial_frame_times_out(self) -> None:
        errors: list[Exception] = []
        with mock.patch.object(
            operator_protocol,
            "DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS",
            0.05,
        ):
            server = listen()
            client: BrokerChannel | None = None

            def serve() -> None:
                try:
                    with server.accept() as channel:
                        channel.receive_json()
                except Exception as exc:  # pragma: no cover - Windows CI
                    errors.append(exc)

            thread = threading.Thread(target=serve)
            thread.start()
            try:
                client = connect(io_timeout=1.0)
                client._transport.send_all(b"J")
                thread.join(2)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], TimeoutError)
            finally:
                if client is not None:
                    client.close()
                server.close()
                thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe timeout test")
    def test_windows_nonreading_client_does_not_block_server_write(self) -> None:
        errors: list[Exception] = []
        with mock.patch.object(
            operator_protocol,
            "DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS",
            0.05,
        ):
            server = listen()
            client: BrokerChannel | None = None

            def serve() -> None:
                try:
                    with server.accept() as channel:
                        channel.send_json({"payload": "x" * (1024 * 1024)})
                except Exception as exc:  # pragma: no cover - Windows CI
                    errors.append(exc)

            thread = threading.Thread(target=serve)
            thread.start()
            try:
                client = connect(io_timeout=1.0)
                thread.join(3)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], TimeoutError)
            finally:
                if client is not None:
                    client.close()
                server.close()
                thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe saturation test")
    def test_windows_listener_survives_more_than_32_connected_instances(self) -> None:
        server = listen()
        accepted: list[BrokerChannel] = []
        clients: list[BrokerChannel] = []
        errors: list[Exception] = []

        def accept_many() -> None:
            try:
                for _ in range(40):
                    accepted.append(server.accept())
            except Exception as exc:  # pragma: no cover - Windows CI
                errors.append(exc)

        thread = threading.Thread(target=accept_many)
        thread.start()
        try:
            for _ in range(40):
                clients.append(connect(timeout=5.0, io_timeout=1.0))
            thread.join(10)
            self.assertFalse(thread.is_alive())
            self.assertEqual(errors, [])
            self.assertEqual(len(accepted), 40)

            result: list[dict] = []

            def serve_one_more() -> None:
                try:
                    with server.accept() as channel:
                        result.append(channel.receive_json())
                        channel.send_json({"ok": True})
                except Exception as exc:  # pragma: no cover - Windows CI
                    errors.append(exc)

            recovery = threading.Thread(target=serve_one_more)
            recovery.start()
            with connect(timeout=5.0, io_timeout=1.0) as channel:
                channel.send_json({"after_saturation": True})
                self.assertEqual(channel.receive_json(), {"ok": True})
            recovery.join(5)
            self.assertFalse(recovery.is_alive())
            self.assertEqual(errors, [])
            self.assertEqual(result, [{"after_saturation": True}])
        finally:
            for channel in clients:
                channel.close()
            for channel in accepted:
                channel.close()
            server.close()
            thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe election test")
    def test_windows_second_listener_loses_election_without_disturbing_first(self) -> None:
        start = threading.Barrier(3)
        listeners: list[operator_protocol._WindowsBrokerListener] = []
        startup_errors: list[Exception] = []
        result: list[dict] = []

        def create_listener() -> None:
            start.wait()
            try:
                listeners.append(listen())
            except Exception as exc:  # pragma: no cover - Windows CI
                startup_errors.append(exc)

        starters = [threading.Thread(target=create_listener) for _ in range(2)]
        for starter in starters:
            starter.start()
        start.wait()
        for starter in starters:
            starter.join(5)

        try:
            self.assertTrue(all(not starter.is_alive() for starter in starters))
            self.assertEqual(len(listeners), 1)
            self.assertEqual(len(startup_errors), 1)
            self.assertIsInstance(startup_errors[0], AppError)
            self.assertEqual(startup_errors[0].code, "operator_broker_running")
            server = listeners[0]

            def serve() -> None:
                with server.accept() as channel:
                    result.append(channel.receive_json())
                    channel.send_json({"winner": True})

            thread = threading.Thread(target=serve)
            thread.start()
            with connect(timeout=5.0, io_timeout=1.0) as channel:
                channel.send_json({"still_running": True})
                self.assertEqual(channel.receive_json(), {"winner": True})
            thread.join(2)
            self.assertFalse(thread.is_alive())
            self.assertEqual(result, [{"still_running": True}])
        finally:
            for listener in listeners:
                listener.close()

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe peer test")
    def test_windows_client_rejects_mismatched_server_sid(self) -> None:
        errors: list[Exception] = []
        with mock.patch.object(
            operator_protocol,
            "DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS",
            0.05,
        ):
            server = listen()

            def serve() -> None:
                try:
                    with server.accept() as channel:
                        channel.receive_json()
                except Exception as exc:  # pragma: no cover - Windows CI
                    errors.append(exc)

            thread = threading.Thread(target=serve)
            thread.start()
            try:
                with mock.patch.object(
                    operator_protocol,
                    "_windows_server_sid",
                    return_value="S-1-5-21-injected-other-user",
                ):
                    with self.assertRaises(AppError) as raised:
                        connect(timeout=5.0, io_timeout=1.0)
                self.assertEqual(raised.exception.code, "operator_peer_rejected")
                thread.join(2)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertNotIsInstance(errors[0], TimeoutError)
            finally:
                server.close()
                thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe peer test")
    def test_windows_server_rejects_mismatched_client_sid(self) -> None:
        errors: list[Exception] = []
        server = listen()
        client: BrokerChannel | None = None

        def serve() -> None:
            try:
                server.accept()
            except Exception as exc:  # pragma: no cover - Windows CI
                errors.append(exc)

        with mock.patch.object(
            operator_protocol,
            "_windows_client_sid",
            return_value="S-1-5-21-injected-other-user",
        ):
            thread = threading.Thread(target=serve)
            thread.start()
            try:
                client = connect(timeout=5.0, io_timeout=0.1)
                thread.join(2)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], AppError)
                self.assertEqual(errors[0].code, "operator_peer_rejected")
                try:
                    client.receive_json()
                except (EOFError, OSError) as exc:
                    self.assertNotIsInstance(exc, TimeoutError)
                else:  # pragma: no cover - defensive Windows assertion
                    self.fail("mismatched named-pipe client remained connected")
            finally:
                if client is not None:
                    client.close()
                server.close()
                thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe peer test")
    def test_windows_client_closes_pipe_when_server_sid_lookup_fails(self) -> None:
        errors: list[Exception] = []
        with mock.patch.object(
            operator_protocol,
            "DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS",
            0.05,
        ):
            server = listen()

            def serve() -> None:
                try:
                    with server.accept() as channel:
                        channel.receive_json()
                except Exception as exc:  # pragma: no cover - Windows CI
                    errors.append(exc)

            thread = threading.Thread(target=serve)
            thread.start()
            try:
                with mock.patch.object(
                    operator_protocol,
                    "_windows_server_sid",
                    side_effect=OSError("injected server SID lookup failure"),
                ):
                    with self.assertRaisesRegex(OSError, "injected server SID"):
                        connect(timeout=5.0, io_timeout=1.0)
                thread.join(2)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertNotIsInstance(errors[0], TimeoutError)
            finally:
                server.close()
                thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe peer test")
    def test_windows_server_closes_pipe_when_client_sid_lookup_fails(self) -> None:
        errors: list[Exception] = []
        server = listen()
        client: BrokerChannel | None = None

        def serve() -> None:
            try:
                server.accept()
            except Exception as exc:  # pragma: no cover - Windows CI
                errors.append(exc)

        with mock.patch.object(
            operator_protocol,
            "_windows_client_sid",
            side_effect=OSError("injected client SID lookup failure"),
        ):
            thread = threading.Thread(target=serve)
            thread.start()
            try:
                client = connect(timeout=5.0, io_timeout=0.1)
                thread.join(2)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertRegex(str(errors[0]), "injected client SID")
                try:
                    client.receive_json()
                except (EOFError, OSError) as exc:
                    self.assertNotIsInstance(exc, TimeoutError)
                else:  # pragma: no cover - defensive Windows assertion
                    self.fail("rejected named-pipe client remained connected")
            finally:
                if client is not None:
                    client.close()
                server.close()
                thread.join(2)

    @unittest.skipUnless(os.name == "nt", "Windows named-pipe close-race test")
    def test_windows_listener_close_does_not_double_close_connected_handle(self) -> None:
        server = listen()
        server_handle = int(server._pending)
        kernel32 = operator_protocol._kernel32
        original_connect = kernel32.ConnectNamedPipe
        original_result = kernel32.GetOverlappedResult
        original_close = kernel32.CloseHandle
        connect_issued = threading.Event()
        connect_completed = threading.Event()
        release_result = threading.Event()
        close_count = 0
        errors: list[Exception] = []
        client: BrokerChannel | None = None

        def controlled_connect(handle: object, overlapped: object) -> int:
            result = original_connect(handle, overlapped)
            error = operator_protocol.ctypes.get_last_error()
            connect_issued.set()
            operator_protocol.ctypes.set_last_error(error)
            return result

        def controlled_result(
            handle: object,
            overlapped: object,
            transferred: object,
            wait: object,
        ) -> int:
            result = original_result(handle, overlapped, transferred, wait)
            error = operator_protocol.ctypes.get_last_error()
            connect_completed.set()
            release_result.wait(5)
            operator_protocol.ctypes.set_last_error(error)
            return result

        def counted_close(handle: object) -> int:
            nonlocal close_count
            value = getattr(handle, "value", handle)
            if value == server_handle:
                close_count += 1
            return original_close(handle)

        def accept_until_closed() -> None:
            try:
                server.accept()
            except Exception as exc:  # pragma: no cover - Windows CI
                errors.append(exc)

        with (
            mock.patch.object(kernel32, "ConnectNamedPipe", side_effect=controlled_connect),
            mock.patch.object(kernel32, "GetOverlappedResult", side_effect=controlled_result),
            mock.patch.object(kernel32, "CloseHandle", side_effect=counted_close),
        ):
            thread = threading.Thread(target=accept_until_closed)
            thread.start()
            try:
                self.assertTrue(connect_issued.wait(2))
                client = connect(timeout=5.0, io_timeout=1.0)
                self.assertTrue(connect_completed.wait(2))
                server.close()
                release_result.set()
                thread.join(2)
                self.assertFalse(thread.is_alive())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], OSError)
                self.assertEqual(close_count, 1)
            finally:
                release_result.set()
                if client is not None:
                    client.close()
                server.close()
                thread.join(2)

    @unittest.skipIf(os.name == "nt", "Unix timeout test")
    def test_ping_read_is_bounded_when_endpoint_accepts_but_never_replies(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch.dict(
            os.environ,
            {
                "KASSIBER_OPERATOR_RUNTIME_DIR": tmp,
                operator_protocol.TEST_RUNTIME_OVERRIDE_ENV: "1",
            },
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
