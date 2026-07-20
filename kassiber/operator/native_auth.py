"""Signed macOS Touch ID helper bridge for the operator-only namespace."""

from __future__ import annotations

import hashlib
import hmac
import os
import secrets
import subprocess
import sys
from pathlib import Path

from ..db import load_managed_settings, update_managed_settings
from ..errors import AppError
from .project import canonical_project
from .policy import require_project_policy_binding
from .service import _wipe


OPERATOR_NATIVE_AUTH_GENERATION_SETTING = "operator_native_auth_generation"


def operator_touch_id_account(data_root: str) -> str:
    project = canonical_project(data_root)
    canonical_data_root = str(project.database.parent)
    # Native credentials are usable only while the adjacent policy remains
    # bound to this exact filesystem identity and an authenticated database ID.
    require_project_policy_binding(canonical_data_root)
    generation = load_managed_settings(canonical_data_root).get(
        OPERATOR_NATIVE_AUTH_GENERATION_SETTING,
        "initial",
    )
    if not isinstance(generation, str):
        generation = "invalid"
    return hashlib.sha256(
        f"kassiber-operator-touch-id-v1:{project.identity}:{generation}".encode("utf-8")
    ).hexdigest()


def invalidate_operator_native_auth(data_root: str) -> str:
    generation = secrets.token_hex(16)
    update_managed_settings(
        data_root,
        updates={OPERATOR_NATIVE_AUTH_GENERATION_SETTING: generation},
    )
    return generation


def broker_touch_id_passphrase(
    data_root: str,
    *,
    expected_helper_identity: str | None = None,
) -> bytearray:
    """Broker-only bridge: the signed helper writes to a broker-created pipe."""

    account = operator_touch_id_account(data_root)
    read_fd, write_fd = os.pipe()
    os.set_inheritable(write_fd, True)
    process: subprocess.Popen[bytes] | None = None
    value = bytearray()
    try:
        process = subprocess.Popen(
            [
                str(_helper_path(expected_helper_identity)),
                "--operator-native-auth",
                "broker-get",
                "--account",
                account,
                "--output-fd",
                str(write_fd),
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            pass_fds=(write_fd,),
        )
        os.close(write_fd)
        write_fd = -1
        value = _read_fd_limited(read_fd)
        stderr = process.stderr.read() if process.stderr is not None else b""
        return_code = process.wait()
    finally:
        if write_fd >= 0:
            os.close(write_fd)
        os.close(read_fd)
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
    if return_code == 4:
        _wipe(value)
        raise AppError(
            "no Touch ID operator credential is enrolled for this project",
            code="operator_native_auth_not_configured",
            hint="Run `kassiber operator touch-id enroll` from the signed macOS app CLI.",
            retryable=False,
        )
    if return_code != 0:
        _wipe(value)
        raise _native_error(stderr)
    if not value:
        raise AppError(
            "the Touch ID helper returned an empty credential",
            code="native_auth_failed",
            retryable=False,
        )
    return value


def touch_id_store(
    data_root: str,
    passphrase: bytearray,
    *,
    expected_helper_identity: str | None = None,
) -> None:
    helper = _helper_path(expected_helper_identity)
    read_fd, write_fd = os.pipe()
    process: subprocess.Popen[bytes] | None = None
    try:
        os.set_inheritable(read_fd, True)
        process = subprocess.Popen(
            [
                str(helper),
                "--operator-native-auth",
                "store",
                "--account",
                operator_touch_id_account(data_root),
                "--secret-fd",
                str(read_fd),
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            pass_fds=(read_fd,),
        )
        os.close(read_fd)
        read_fd = -1
        _write_fd(write_fd, passphrase)
        os.close(write_fd)
        write_fd = -1
        stderr = process.stderr.read() if process.stderr is not None else b""
        if process.wait() != 0:
            raise _native_error(stderr)
    finally:
        if write_fd >= 0:
            os.close(write_fd)
        if read_fd >= 0:
            os.close(read_fd)
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()


def touch_id_delete(
    data_root: str,
    *,
    expected_helper_identity: str | None = None,
) -> None:
    completed = _run_no_secret(
        "delete",
        data_root,
        expected_helper_identity=expected_helper_identity,
    )
    if completed.returncode != 0:
        raise _native_error(completed.stderr)


def touch_id_status(data_root: str) -> dict[str, object]:
    try:
        completed = _run_no_secret("status", data_root)
    except AppError as exc:
        return {"available": False, "configured": False, "reason": exc.code}
    if completed.returncode == 0:
        return {"available": True, "configured": True}
    if completed.returncode == 4:
        return {"available": True, "configured": False}
    return {"available": False, "configured": False, "reason": "native_auth_failed"}


def native_auth_runtime_available() -> bool:
    """Report whether this broker inherited a usable native helper path."""

    try:
        _helper_path()
    except AppError:
        return False
    return True


def native_auth_helper_identity() -> str:
    """Return a public-safe identity for the exact configured helper file."""

    return _helper_identity(_helper_path())


def validate_native_auth_helper_identity(expected_identity: str) -> None:
    """Fail unless the currently configured helper matches the signed caller."""

    _helper_path(expected_identity)


def _run_no_secret(
    action: str,
    data_root: str,
    *,
    expected_helper_identity: str | None = None,
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [
            str(_helper_path(expected_helper_identity)),
            "--operator-native-auth",
            action,
            "--account",
            operator_touch_id_account(data_root),
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
    )


def _helper_path(expected_identity: str | None = None) -> Path:
    configured = os.environ.get("KASSIBER_NATIVE_AUTH_HELPER")
    if sys.platform != "darwin" or not configured:
        raise AppError(
            "Touch ID operator authentication is unavailable in this build",
            code="native_auth_unavailable",
            hint="Use password authentication or the signed macOS desktop CLI launcher.",
            retryable=False,
        )
    helper = Path(configured).expanduser().resolve(strict=False)
    if not helper.is_file() or not os.access(helper, os.X_OK):
        raise AppError(
            "the native authentication helper is unavailable",
            code="native_auth_unavailable",
            retryable=False,
        )
    if expected_identity is not None and not hmac.compare_digest(
        _helper_identity(helper),
        expected_identity,
    ):
        raise AppError(
            "the broker native-auth helper does not match this signed CLI",
            code="native_auth_helper_mismatch",
            hint="Retry from the signed macOS app CLI after stopping idle broker work.",
            retryable=True,
        )
    return helper


def _helper_identity(helper: Path) -> str:
    digest = hashlib.sha256()
    digest.update(b"kassiber-native-auth-helper-v1\0")
    digest.update(os.fsencode(helper))
    try:
        with helper.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                digest.update(chunk)
    except OSError as exc:
        raise AppError(
            "the native authentication helper cannot be inspected",
            code="native_auth_unavailable",
            retryable=False,
        ) from exc
    return digest.hexdigest()


def _native_error(stderr: bytes) -> AppError:
    message = stderr.decode("utf-8", errors="replace").strip()
    return AppError(
        message or "Touch ID authentication failed",
        code="native_auth_failed",
        retryable=False,
    )


def _read_fd_limited(fd: int) -> bytearray:
    value = bytearray()
    while True:
        chunk = os.read(fd, min(4096, 16 * 1024 + 1 - len(value)))
        if not chunk:
            break
        value.extend(chunk)
        if len(value) > 16 * 1024:
            _wipe(value)
            raise AppError(
                "native authentication secret exceeds 16 KiB",
                code="native_auth_failed",
            )
    return value


def _write_fd(fd: int, value: bytearray) -> None:
    view = memoryview(value)
    written = 0
    while written < len(view):
        written += os.write(fd, view[written:])
