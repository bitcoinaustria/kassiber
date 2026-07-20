"""Authenticated local IPC and secret-separated broker framing."""

from __future__ import annotations

import ctypes
import hashlib
import json
import os
import socket
import stat
import struct
import sys
import time
from pathlib import Path
from typing import Any, BinaryIO, Protocol

from ..errors import AppError


PROTOCOL_VERSION = 1
MAX_JSON_FRAME = 8 * 1024 * 1024
MAX_SECRET_FRAME = 16 * 1024
DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS = 30.0
DEFAULT_UNIX_SERVER_IO_TIMEOUT_SECONDS = 30.0
SOCKET_FILENAME = "operator-v1.sock"
STARTUP_LOCK_FILENAME = "operator-v1.start.lock"
_HEADER = struct.Struct("!cI")


class _ReadableWritable(Protocol):
    def recv_exact(self, size: int) -> bytes: ...
    def send_all(self, payload: bytes) -> None: ...
    def close(self) -> None: ...


class BrokerChannel:
    """Length-prefixed frames with a distinct non-JSON secret frame type."""

    def __init__(self, transport: _ReadableWritable) -> None:
        self._transport = transport

    def send_json(self, payload: dict[str, Any]) -> None:
        raw = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
        if len(raw) > MAX_JSON_FRAME:
            raise AppError("operator request is too large", code="operator_frame_too_large")
        self._transport.send_all(_HEADER.pack(b"J", len(raw)) + raw)

    def receive_json(self) -> dict[str, Any]:
        kind, raw = self._receive_frame(MAX_JSON_FRAME)
        if kind != b"J":
            raise AppError(
                "expected operator protocol JSON frame",
                code="operator_protocol_error",
                retryable=False,
            )
        try:
            payload = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise AppError(
                "invalid operator protocol JSON",
                code="operator_protocol_error",
                retryable=False,
            ) from exc
        if not isinstance(payload, dict):
            raise AppError(
                "operator protocol payload must be an object",
                code="operator_protocol_error",
                retryable=False,
            )
        return payload

    def send_secret(self, challenge: str, secret: str | bytes) -> None:
        challenge_bytes = challenge.encode("ascii")
        secret_bytes = secret.encode("utf-8") if isinstance(secret, str) else bytes(secret)
        if not challenge_bytes or b"\0" in challenge_bytes:
            raise ValueError("invalid challenge")
        raw = bytearray(challenge_bytes + b"\0" + secret_bytes)
        if len(raw) > MAX_SECRET_FRAME:
            _wipe(raw)
            raise AppError("operator secret is too large", code="operator_secret_too_large")
        try:
            self._transport.send_all(_HEADER.pack(b"S", len(raw)) + raw)
        finally:
            _wipe(raw)

    def receive_secret(self, expected_challenge: str) -> bytearray:
        kind, raw = self._receive_frame(MAX_SECRET_FRAME)
        if kind != b"S":
            raise AppError(
                "expected operator protocol secret frame",
                code="operator_protocol_error",
                retryable=False,
            )
        challenge, separator, secret = raw.partition(b"\0")
        if not separator or not _constant_time_equal(
            challenge, expected_challenge.encode("ascii")
        ):
            raise AppError(
                "operator secret challenge did not match",
                code="operator_secret_challenge_mismatch",
                retryable=False,
            )
        return bytearray(secret)

    def _receive_frame(self, limit: int) -> tuple[bytes, bytes]:
        header = self._transport.recv_exact(_HEADER.size)
        kind, size = _HEADER.unpack(header)
        if size > limit:
            raise AppError(
                "operator protocol frame exceeds its limit",
                code="operator_frame_too_large",
                retryable=False,
            )
        return kind, self._transport.recv_exact(size)

    def close(self) -> None:
        self._transport.close()

    def __enter__(self) -> BrokerChannel:
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()


class _SocketTransport:
    def __init__(self, connection: socket.socket) -> None:
        self.connection = connection

    def recv_exact(self, size: int) -> bytes:
        chunks = bytearray()
        while len(chunks) < size:
            part = self.connection.recv(size - len(chunks))
            if not part:
                raise EOFError("operator IPC connection closed")
            chunks.extend(part)
        return bytes(chunks)

    def send_all(self, payload: bytes) -> None:
        self.connection.sendall(payload)

    def close(self) -> None:
        self.connection.close()


class UnixBrokerListener:
    def __init__(self, endpoint: Path, startup_lock: BinaryIO) -> None:
        self.endpoint = endpoint
        self._startup_lock = startup_lock
        self._socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._closed = False
        self._endpoint_identity: tuple[int, int] | None = None
        try:
            self._socket.bind(str(endpoint))
            os.chmod(endpoint, 0o600)
            self._socket.listen(32)
            info = endpoint.lstat()
            self._endpoint_identity = (info.st_dev, info.st_ino)
        except Exception:
            self._socket.close()
            self._startup_lock.close()
            raise

    def accept(self) -> BrokerChannel:
        connection, _ = self._socket.accept()
        try:
            _verify_unix_peer(connection)
            connection.settimeout(DEFAULT_UNIX_SERVER_IO_TIMEOUT_SECONDS)
        except Exception:
            connection.close()
            raise
        return BrokerChannel(_SocketTransport(connection))

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._socket.close()
        try:
            info = self.endpoint.lstat()
            if self._endpoint_identity == (info.st_dev, info.st_ino):
                self.endpoint.unlink()
        except FileNotFoundError:
            pass
        finally:
            self._startup_lock.close()


def operator_runtime_dir() -> Path:
    override = os.environ.get("KASSIBER_OPERATOR_RUNTIME_DIR")
    if override:
        selected = Path(override).expanduser()
    else:
        xdg = os.environ.get("XDG_RUNTIME_DIR")
        selected = Path(xdg) / "kassiber" if xdg else Path.home() / ".kassiber" / "run"
    _ensure_private_directory(selected)
    return selected.resolve(strict=True)


def operator_endpoint() -> Path | str:
    if os.name == "nt":
        sid = _windows_current_sid()
        digest = hashlib.sha256(sid.encode("utf-8")).hexdigest()[:24]
        return rf"\\.\pipe\kassiber-operator-{digest}"
    return operator_runtime_dir() / SOCKET_FILENAME


def listen() -> UnixBrokerListener | _WindowsBrokerListener:
    endpoint = operator_endpoint()
    if os.name == "nt":
        return _WindowsBrokerListener(str(endpoint))
    assert isinstance(endpoint, Path)
    startup_lock = _acquire_startup_lock(endpoint.parent / STARTUP_LOCK_FILENAME)
    try:
        if endpoint.exists() or endpoint.is_symlink():
            _remove_stale_socket(endpoint)
        return UnixBrokerListener(endpoint, startup_lock)
    except Exception:
        startup_lock.close()
        raise


def connect(
    *,
    timeout: float = 2.0,
    io_timeout: float | None = None,
) -> BrokerChannel:
    endpoint = operator_endpoint()
    if os.name == "nt":
        return BrokerChannel(
            _WindowsPipeTransport.connect(
                str(endpoint),
                timeout=timeout,
                io_timeout=(
                    DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS
                    if io_timeout is None
                    else io_timeout
                ),
            )
        )
    assert isinstance(endpoint, Path)
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.settimeout(timeout)
    try:
        connection.connect(str(endpoint))
        _verify_unix_peer(connection)
    except Exception:
        connection.close()
        raise
    connection.settimeout(io_timeout)
    return BrokerChannel(_SocketTransport(connection))


def _acquire_startup_lock(path: Path) -> BinaryIO:
    import fcntl

    flags = os.O_RDWR | os.O_CREAT
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    try:
        fd = os.open(path, flags, 0o600)
    except OSError as exc:
        raise AppError(
            "the operator startup lock is unavailable",
            code="operator_runtime_unavailable",
            retryable=True,
        ) from exc
    handle = os.fdopen(fd, "r+b", buffering=0)
    try:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise AppError(
                "the operator startup lock is unsafe",
                code="unsafe_operator_endpoint",
                retryable=False,
            )
        if hasattr(os, "getuid") and info.st_uid != os.getuid():
            raise AppError(
                "the operator startup lock belongs to another OS user",
                code="unsafe_operator_endpoint",
                retryable=False,
            )
        os.fchmod(handle.fileno(), 0o600)
        try:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise AppError(
                "the operator broker is already starting or running",
                code="operator_broker_running",
                retryable=False,
            ) from exc
        return handle
    except Exception:
        handle.close()
        raise


def _ensure_private_directory(path: Path) -> None:
    try:
        if path.is_symlink():
            raise AppError(
                "operator runtime directory may not be a symlink",
                code="unsafe_operator_runtime_directory",
                retryable=False,
            )
        path.mkdir(parents=True, mode=0o700, exist_ok=True)
        info = path.stat()
    except OSError as exc:
        raise AppError(
            "operator runtime directory is unavailable",
            code="operator_runtime_unavailable",
            retryable=True,
        ) from exc
    if not stat.S_ISDIR(info.st_mode):
        raise AppError(
            "operator runtime path is not a directory",
            code="unsafe_operator_runtime_directory",
            retryable=False,
        )
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        raise AppError(
            "operator runtime directory is owned by another OS user",
            code="unsafe_operator_runtime_directory",
            retryable=False,
        )
    if os.name != "nt" and info.st_mode & 0o077:
        raise AppError(
            "operator runtime directory permissions are too broad",
            code="unsafe_operator_runtime_directory",
            hint="Restrict the directory to mode 0700 and retry.",
            retryable=False,
        )


def _remove_stale_socket(endpoint: Path) -> None:
    info = endpoint.lstat()
    if not stat.S_ISSOCK(info.st_mode):
        raise AppError(
            "operator endpoint is not a socket",
            code="unsafe_operator_endpoint",
            retryable=False,
        )
    if hasattr(os, "getuid") and info.st_uid != os.getuid():
        raise AppError(
            "operator endpoint is owned by another OS user",
            code="unsafe_operator_endpoint",
            retryable=False,
        )
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    probe.settimeout(0.2)
    try:
        probe.connect(str(endpoint))
    except (ConnectionRefusedError, FileNotFoundError):
        endpoint.unlink()
        return
    finally:
        probe.close()
    raise AppError(
        "the operator broker is already running",
        code="operator_broker_running",
        retryable=False,
    )


def _verify_unix_peer(connection: socket.socket) -> None:
    expected_uid = os.getuid()
    if sys.platform.startswith("linux"):
        raw = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        _pid, uid, _gid = struct.unpack("3i", raw)
    elif sys.platform == "darwin":
        uid = ctypes.c_uint()
        gid = ctypes.c_uint()
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.getpeereid(connection.fileno(), ctypes.byref(uid), ctypes.byref(gid)) != 0:
            raise OSError(ctypes.get_errno(), "getpeereid failed")
        uid = uid.value
    else:
        raise AppError(
            "this platform lacks a supported local peer-credential primitive",
            code="operator_peer_credentials_unavailable",
            retryable=False,
        )
    if uid != expected_uid:
        raise AppError(
            "operator IPC peer is a different OS user",
            code="operator_peer_rejected",
            retryable=False,
        )


def _constant_time_equal(left: bytes, right: bytes) -> bool:
    import hmac

    return hmac.compare_digest(left, right)


def _wipe(value: bytearray) -> None:
    for index in range(len(value)):
        value[index] = 0


# Windows uses a real local named pipe.  Its protected DACL grants access only
# to the current SID, PIPE_REJECT_REMOTE_CLIENTS rejects network clients, and
# every accepted client's token SID is compared again before framing begins.
if os.name == "nt":  # pragma: no cover - exercised by the Windows CI job
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    _INVALID_HANDLE_VALUE = wintypes.HANDLE(-1).value
    _TOKEN_QUERY = 0x0008
    _TOKEN_USER = 1
    _PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    _PIPE_ACCESS_DUPLEX = 0x00000003
    _FILE_FLAG_FIRST_PIPE_INSTANCE = 0x00080000
    _PIPE_TYPE_BYTE = 0x00000000
    _PIPE_READMODE_BYTE = 0x00000000
    _PIPE_WAIT = 0x00000000
    _PIPE_REJECT_REMOTE_CLIENTS = 0x00000008
    _OPEN_EXISTING = 3
    _GENERIC_READ = 0x80000000
    _GENERIC_WRITE = 0x40000000
    _ERROR_PIPE_CONNECTED = 535
    _ERROR_PIPE_BUSY = 231
    _SDDL_REVISION_1 = 1
    _SE_FILE_OBJECT = 1
    _OWNER_SECURITY_INFORMATION = 0x00000001

    class _SECURITY_ATTRIBUTES(ctypes.Structure):
        _fields_ = [
            ("nLength", wintypes.DWORD),
            ("lpSecurityDescriptor", wintypes.LPVOID),
            ("bInheritHandle", wintypes.BOOL),
        ]

    _kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    _kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    _kernel32.CloseHandle.restype = wintypes.BOOL
    _kernel32.LocalFree.argtypes = [wintypes.LPVOID]
    _kernel32.LocalFree.restype = wintypes.LPVOID
    _kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    ]
    _kernel32.CreateFileW.restype = wintypes.HANDLE
    _kernel32.CreateNamedPipeW.restype = wintypes.HANDLE
    _kernel32.CreateNamedPipeW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.POINTER(_SECURITY_ATTRIBUTES),
    ]
    _kernel32.ConnectNamedPipe.argtypes = [wintypes.HANDLE, wintypes.LPVOID]
    _kernel32.ConnectNamedPipe.restype = wintypes.BOOL
    _kernel32.GetNamedPipeClientProcessId.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(wintypes.ULONG),
    ]
    _kernel32.GetNamedPipeClientProcessId.restype = wintypes.BOOL
    _kernel32.GetNamedPipeServerProcessId.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(wintypes.ULONG),
    ]
    _kernel32.GetNamedPipeServerProcessId.restype = wintypes.BOOL
    _kernel32.DisconnectNamedPipe.argtypes = [wintypes.HANDLE]
    _kernel32.DisconnectNamedPipe.restype = wintypes.BOOL
    _kernel32.WaitNamedPipeW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
    _kernel32.WaitNamedPipeW.restype = wintypes.BOOL
    _kernel32.ReadFile.argtypes = [
        wintypes.HANDLE,
        wintypes.LPVOID,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
        wintypes.LPVOID,
    ]
    _kernel32.ReadFile.restype = wintypes.BOOL
    _kernel32.PeekNamedPipe.argtypes = [
        wintypes.HANDLE,
        wintypes.LPVOID,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
        ctypes.POINTER(wintypes.DWORD),
        ctypes.POINTER(wintypes.DWORD),
    ]
    _kernel32.PeekNamedPipe.restype = wintypes.BOOL
    _kernel32.WriteFile.argtypes = [
        wintypes.HANDLE,
        wintypes.LPCVOID,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
        wintypes.LPVOID,
    ]
    _kernel32.WriteFile.restype = wintypes.BOOL
    _advapi32.OpenProcessToken.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.HANDLE),
    ]
    _advapi32.OpenProcessToken.restype = wintypes.BOOL
    _advapi32.GetTokenInformation.restype = wintypes.BOOL
    _advapi32.GetTokenInformation.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
    ]
    _advapi32.ConvertSidToStringSidW.argtypes = [
        wintypes.LPVOID,
        ctypes.POINTER(wintypes.LPWSTR),
    ]
    _advapi32.ConvertSidToStringSidW.restype = wintypes.BOOL
    _advapi32.ConvertStringSecurityDescriptorToSecurityDescriptorW.restype = (
        wintypes.BOOL
    )
    _advapi32.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.DWORD),
    ]
    _advapi32.GetNamedSecurityInfoW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.LPVOID),
    ]
    _advapi32.GetNamedSecurityInfoW.restype = wintypes.DWORD

    def _raise_windows(message: str) -> None:
        raise OSError(ctypes.get_last_error(), message)

    def _windows_current_sid() -> str:
        process = _kernel32.GetCurrentProcess()
        token = wintypes.HANDLE()
        if not _advapi32.OpenProcessToken(process, _TOKEN_QUERY, ctypes.byref(token)):
            _raise_windows("OpenProcessToken failed")
        try:
            return _windows_token_sid(token)
        finally:
            _kernel32.CloseHandle(token)

    def _windows_token_sid(token: wintypes.HANDLE) -> str:
        needed = wintypes.DWORD()
        _advapi32.GetTokenInformation(token, _TOKEN_USER, None, 0, ctypes.byref(needed))
        buffer = ctypes.create_string_buffer(needed.value)
        if not _advapi32.GetTokenInformation(
            token, _TOKEN_USER, buffer, needed, ctypes.byref(needed)
        ):
            _raise_windows("GetTokenInformation failed")
        sid_pointer = ctypes.cast(buffer, ctypes.POINTER(wintypes.LPVOID))[0]
        text = wintypes.LPWSTR()
        if not _advapi32.ConvertSidToStringSidW(sid_pointer, ctypes.byref(text)):
            _raise_windows("ConvertSidToStringSidW failed")
        try:
            return text.value
        finally:
            _kernel32.LocalFree(ctypes.cast(text, wintypes.LPVOID))

    def windows_path_owned_by_current_user(path: str) -> bool:
        owner = wintypes.LPVOID()
        descriptor = wintypes.LPVOID()
        result = _advapi32.GetNamedSecurityInfoW(
            path,
            _SE_FILE_OBJECT,
            _OWNER_SECURITY_INFORMATION,
            ctypes.byref(owner),
            None,
            None,
            None,
            ctypes.byref(descriptor),
        )
        if result != 0:
            raise OSError(result, "GetNamedSecurityInfoW failed")
        text = wintypes.LPWSTR()
        try:
            if not _advapi32.ConvertSidToStringSidW(owner, ctypes.byref(text)):
                _raise_windows("ConvertSidToStringSidW for path owner failed")
            try:
                return text.value == _windows_current_sid()
            finally:
                _kernel32.LocalFree(ctypes.cast(text, wintypes.LPVOID))
        finally:
            _kernel32.LocalFree(descriptor)

    def _windows_client_sid(pipe: wintypes.HANDLE) -> str:
        pid = wintypes.ULONG()
        if not _kernel32.GetNamedPipeClientProcessId(pipe, ctypes.byref(pid)):
            _raise_windows("GetNamedPipeClientProcessId failed")
        process = _kernel32.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid.value)
        if not process:
            _raise_windows("OpenProcess for named-pipe client failed")
        token = wintypes.HANDLE()
        try:
            if not _advapi32.OpenProcessToken(process, _TOKEN_QUERY, ctypes.byref(token)):
                _raise_windows("OpenProcessToken for named-pipe client failed")
            try:
                return _windows_token_sid(token)
            finally:
                _kernel32.CloseHandle(token)
        finally:
            _kernel32.CloseHandle(process)

    def _windows_server_sid(pipe: wintypes.HANDLE) -> str:
        pid = wintypes.ULONG()
        if not _kernel32.GetNamedPipeServerProcessId(pipe, ctypes.byref(pid)):
            _raise_windows("GetNamedPipeServerProcessId failed")
        process = _kernel32.OpenProcess(_PROCESS_QUERY_LIMITED_INFORMATION, False, pid.value)
        if not process:
            _raise_windows("OpenProcess for named-pipe server failed")
        token = wintypes.HANDLE()
        try:
            if not _advapi32.OpenProcessToken(process, _TOKEN_QUERY, ctypes.byref(token)):
                _raise_windows("OpenProcessToken for named-pipe server failed")
            try:
                return _windows_token_sid(token)
            finally:
                _kernel32.CloseHandle(token)
        finally:
            _kernel32.CloseHandle(process)

    class _WindowsPipeTransport:
        def __init__(
            self,
            handle: wintypes.HANDLE,
            *,
            io_timeout: float | None = None,
        ) -> None:
            self.handle = handle
            self.io_timeout = io_timeout

        @classmethod
        def connect(
            cls,
            endpoint: str,
            *,
            timeout: float,
            io_timeout: float | None,
        ) -> _WindowsPipeTransport:
            deadline = time.monotonic() + timeout
            while True:
                handle = _kernel32.CreateFileW(
                    endpoint,
                    _GENERIC_READ | _GENERIC_WRITE,
                    0,
                    None,
                    _OPEN_EXISTING,
                    0,
                    None,
                )
                if handle != _INVALID_HANDLE_VALUE:
                    if _windows_server_sid(handle) != _windows_current_sid():
                        _kernel32.CloseHandle(handle)
                        raise AppError(
                            "operator named-pipe server SID did not match",
                            code="operator_peer_rejected",
                            retryable=False,
                        )
                    return cls(handle, io_timeout=io_timeout)
                error = ctypes.get_last_error()
                if error != _ERROR_PIPE_BUSY or time.monotonic() >= deadline:
                    raise OSError(error, "could not connect to operator named pipe")
                _kernel32.WaitNamedPipeW(endpoint, 100)

        def recv_exact(self, size: int) -> bytes:
            chunks = bytearray()
            deadline = (
                time.monotonic() + self.io_timeout
                if self.io_timeout is not None
                else None
            )
            while len(chunks) < size:
                read_size = size - len(chunks)
                if deadline is not None:
                    available = wintypes.DWORD()
                    while available.value == 0:
                        if not _kernel32.PeekNamedPipe(
                            self.handle,
                            None,
                            0,
                            None,
                            ctypes.byref(available),
                            None,
                        ):
                            _raise_windows("PeekNamedPipe from operator pipe failed")
                        if available.value:
                            break
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError("operator named-pipe read timed out")
                        time.sleep(min(0.01, remaining))
                    read_size = min(read_size, available.value)
                buffer = ctypes.create_string_buffer(read_size)
                read = wintypes.DWORD()
                if not _kernel32.ReadFile(
                    self.handle, buffer, len(buffer), ctypes.byref(read), None
                ):
                    _raise_windows("ReadFile from operator pipe failed")
                if read.value == 0:
                    raise EOFError("operator named pipe closed")
                chunks.extend(buffer.raw[: read.value])
            return bytes(chunks)

        def send_all(self, payload: bytes) -> None:
            offset = 0
            while offset < len(payload):
                written = wintypes.DWORD()
                chunk = payload[offset:]
                buffer = ctypes.create_string_buffer(chunk)
                if not _kernel32.WriteFile(
                    self.handle, buffer, len(chunk), ctypes.byref(written), None
                ):
                    _raise_windows("WriteFile to operator pipe failed")
                offset += written.value

        def close(self) -> None:
            _kernel32.CloseHandle(self.handle)

    class _WindowsBrokerListener:
        def __init__(self, endpoint: str) -> None:
            self.endpoint = endpoint
            self._first = True
            self._closed = False
            self._pending = self._create_instance(first=True)

        def _create_instance(self, *, first: bool) -> wintypes.HANDLE:
            sid = _windows_current_sid()
            descriptor = wintypes.LPVOID()
            sddl = f"D:P(A;;GA;;;{sid})"
            if not _advapi32.ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl, _SDDL_REVISION_1, ctypes.byref(descriptor), None
            ):
                _raise_windows("could not construct operator pipe DACL")
            attributes = _SECURITY_ATTRIBUTES(
                ctypes.sizeof(_SECURITY_ATTRIBUTES), descriptor, False
            )
            try:
                access = _PIPE_ACCESS_DUPLEX
                if first:
                    access |= _FILE_FLAG_FIRST_PIPE_INSTANCE
                handle = _kernel32.CreateNamedPipeW(
                    self.endpoint,
                    access,
                    _PIPE_TYPE_BYTE
                    | _PIPE_READMODE_BYTE
                    | _PIPE_WAIT
                    | _PIPE_REJECT_REMOTE_CLIENTS,
                    32,
                    65536,
                    65536,
                    0,
                    ctypes.byref(attributes),
                )
            finally:
                _kernel32.LocalFree(descriptor)
            if handle == _INVALID_HANDLE_VALUE:
                _raise_windows("could not create operator named pipe")
            return handle

        def accept(self) -> BrokerChannel:
            handle = self._pending
            connected = _kernel32.ConnectNamedPipe(handle, None)
            if not connected and ctypes.get_last_error() != _ERROR_PIPE_CONNECTED:
                _kernel32.CloseHandle(handle)
                _raise_windows("ConnectNamedPipe failed")
            self._pending = self._create_instance(first=False)
            if _windows_client_sid(handle) != _windows_current_sid():
                _kernel32.DisconnectNamedPipe(handle)
                _kernel32.CloseHandle(handle)
                raise AppError(
                    "operator named-pipe client SID did not match",
                    code="operator_peer_rejected",
                    retryable=False,
                )
            return BrokerChannel(
                _WindowsPipeTransport(
                    handle,
                    io_timeout=DEFAULT_WINDOWS_IO_TIMEOUT_SECONDS,
                )
            )

        def close(self) -> None:
            if self._closed:
                return
            self._closed = True
            _kernel32.CloseHandle(self._pending)

else:
    def _windows_current_sid() -> str:
        raise RuntimeError("Windows SID lookup is unavailable on this platform")

    class _WindowsPipeTransport:  # pragma: no cover - type placeholder
        pass

    class _WindowsBrokerListener:  # pragma: no cover - type placeholder
        pass

    def windows_path_owned_by_current_user(_path: str) -> bool:
        return True
