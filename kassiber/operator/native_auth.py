"""Signed macOS Touch ID helper bridge for the operator-only namespace."""

from __future__ import annotations

import hashlib
import hmac
import os
import secrets
import select
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from ..db import load_managed_settings, update_managed_settings
from ..errors import AppError
from .project import canonical_project
from .policy import require_project_policy_binding
from .service import _wipe


OPERATOR_NATIVE_AUTH_GENERATION_SETTING = "operator_native_auth_generation"
_NATIVE_AUTH_IDENTIFIER = "at.bitcoinaustria.kassiber"


@dataclass(frozen=True)
class _CodeIdentity:
    identifier: str
    team: str
    cdhash: str

    @property
    def opaque(self) -> str:
        return hashlib.sha256(
            (
                "kassiber-native-auth-signature-v1:"
                f"{self.identifier}:{self.team}:{self.cdhash}"
            ).encode("ascii")
        ).hexdigest()

    @property
    def requirement(self) -> str:
        return (
            f'anchor apple generic and identifier "{self.identifier}" '
            f'and certificate leaf[subject.OU] = "{self.team}" '
            "and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
        )


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
        process = _spawn_validated_helper(
            [
                str(_helper_path(expected_helper_identity)),
                "--operator-native-auth",
                "broker-get",
                "--account",
                account,
                "--output-fd",
                str(write_fd),
            ],
            inherited_fds=(write_fd,),
            expected_identity=expected_helper_identity,
        )
        os.close(write_fd)
        write_fd = -1
        _read_fd_limited(read_fd, value)
        stderr = process.stderr.read() if process.stderr is not None else b""
        return_code = process.wait()
    except BaseException:
        # A secret may already have crossed the pipe when process inspection
        # or collection fails. Wipe the single caller-owned buffer before the
        # original control-flow or operational exception escapes.
        _wipe(value)
        raise
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
        process = _spawn_validated_helper(
            [
                str(helper),
                "--operator-native-auth",
                "store",
                "--account",
                operator_touch_id_account(data_root),
                "--secret-fd",
                str(read_fd),
            ],
            inherited_fds=(read_fd,),
            expected_identity=expected_helper_identity,
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
    """Return a public-safe identity for the signed configured helper."""

    return _signed_helper_identity(_configured_helper_path())


def native_auth_caller_identity() -> str:
    """Bind a CLI invocation to its live signed macOS launcher."""

    helper = _signed_helper_code(_configured_helper_path())
    parent = _inspect_code(str(os.getppid()), requirement=helper.requirement)
    if parent != helper:
        raise AppError(
            "the native-auth helper does not match the live signed CLI launcher",
            code="native_auth_helper_mismatch",
            retryable=False,
        )
    return helper.opaque


def validate_native_auth_helper_identity(expected_identity: str) -> None:
    """Fail unless the currently configured helper matches the signed caller."""

    _helper_path(expected_identity)


def _run_no_secret(
    action: str,
    data_root: str,
    *,
    expected_helper_identity: str | None = None,
) -> subprocess.CompletedProcess[bytes]:
    command = [
        str(_helper_path(expected_helper_identity)),
        "--operator-native-auth",
        action,
        "--account",
        operator_touch_id_account(data_root),
    ]
    process = _spawn_validated_helper(
        command,
        inherited_fds=(),
        expected_identity=expected_helper_identity,
    )
    try:
        stderr = process.stderr.read() if process.stderr is not None else b""
        return_code = process.wait()
        return subprocess.CompletedProcess(command, return_code, b"", stderr)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def _helper_path(expected_identity: str | None = None) -> Path:
    helper = _configured_helper_path()
    actual_identity = _signed_helper_identity(helper)
    if expected_identity is not None and not hmac.compare_digest(
        actual_identity, expected_identity
    ):
        raise AppError(
            "the broker native-auth helper does not match this signed CLI",
            code="native_auth_helper_mismatch",
            hint="Retry from the signed macOS app CLI after stopping idle broker work.",
            retryable=True,
        )
    return helper


def _configured_helper_path() -> Path:
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
    return helper


def _signed_helper_identity(helper: Path) -> str:
    return _signed_helper_code(helper).opaque


def _signed_helper_code(helper: Path) -> _CodeIdentity:
    bundle = next(
        (
            ancestor
            for ancestor in helper.parents
            if ancestor.suffix.lower() == ".app"
        ),
        None,
    )
    expected = bundle / "Contents" / "MacOS" / "Kassiber" if bundle else None
    if expected is None or helper != expected:
        raise AppError(
            "the native authentication helper is outside the Kassiber app bundle",
            code="native_auth_unavailable",
            retryable=False,
        )
    return _inspect_code(str(helper))


def _inspect_code(
    target: str,
    *,
    requirement: str | None = None,
) -> _CodeIdentity:
    details = subprocess.run(
        ["/usr/bin/codesign", "-dv", "--verbose=4", target],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
    )
    if details.returncode != 0:
        raise AppError(
            "the native authentication helper has no valid code signature",
            code="native_auth_unavailable",
            retryable=False,
        )
    fields = _codesign_fields(details.stderr)
    identity = _CodeIdentity(
        identifier=fields.get("Identifier", ""),
        team=fields.get("TeamIdentifier", ""),
        cdhash=fields.get("CDHash", "").lower(),
    )
    if (
        identity.identifier != _NATIVE_AUTH_IDENTIFIER
        or len(identity.team) != 10
        or not identity.team.isascii()
        or not identity.team.isalnum()
        or not identity.team.upper() == identity.team
        or len(identity.cdhash) not in {40, 64}
        or any(value not in "0123456789abcdef" for value in identity.cdhash)
    ):
        raise AppError(
            "the native authentication helper has an unexpected signing identity",
            code="native_auth_unavailable",
            retryable=False,
        )
    required = requirement or identity.requirement
    verified = subprocess.run(
        [
            "/usr/bin/codesign",
            "--verify",
            "--strict",
            "--verbose=2",
            f"-R={required}",
            target,
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
    )
    if verified.returncode != 0:
        raise AppError(
            "the native authentication helper failed dynamic signature validation",
            code="native_auth_unavailable",
            retryable=False,
        )
    return identity


def _codesign_fields(stderr: bytes) -> dict[str, str]:
    fields: dict[str, str] = {}
    for raw_line in stderr.decode("utf-8", errors="replace").splitlines():
        key, separator, value = raw_line.partition("=")
        if separator and key in {"Identifier", "TeamIdentifier", "CDHash"}:
            fields[key] = value.strip()
    return fields


def _validate_spawned_helper(
    process: subprocess.Popen[bytes],
    expected_identity: str | None,
) -> None:
    helper = _signed_helper_code(_helper_path(expected_identity))
    running = _inspect_code(str(process.pid), requirement=helper.requirement)
    if running != helper or (
        expected_identity is not None
        and not hmac.compare_digest(running.opaque, expected_identity)
    ):
        raise AppError(
            "the running native-auth helper does not match the signed CLI",
            code="native_auth_helper_mismatch",
            retryable=False,
        )


def _spawn_validated_helper(
    command: list[str],
    *,
    inherited_fds: tuple[int, ...],
    expected_identity: str | None,
) -> subprocess.Popen[bytes]:
    """Hold the helper behind a pipe gate until its live PID is verified."""

    ready_read, ready_write = os.pipe()
    go_read, go_write = os.pipe()
    process: subprocess.Popen[bytes] | None = None
    try:
        os.set_inheritable(ready_write, True)
        os.set_inheritable(go_read, True)
        gated_command = [
            *command,
            "--ready-fd",
            str(ready_write),
            "--go-fd",
            str(go_read),
        ]
        process = subprocess.Popen(
            gated_command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            pass_fds=(*inherited_fds, ready_write, go_read),
        )
        os.close(ready_write)
        ready_write = -1
        os.close(go_read)
        go_read = -1
        readable, _, _ = select.select([ready_read], [], [], 5.0)
        if not readable or os.read(ready_read, 1) != b"R":
            raise AppError(
                "the native-auth helper did not reach its signed launch gate",
                code="native_auth_failed",
                retryable=False,
            )
        _validate_spawned_helper(process, expected_identity)
        os.write(go_write, b"G")
        os.close(go_write)
        go_write = -1
        return process
    except BaseException:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        raise
    finally:
        for fd in (ready_read, ready_write, go_read, go_write):
            if fd >= 0:
                try:
                    os.close(fd)
                except OSError:
                    pass


def _native_error(stderr: bytes) -> AppError:
    message = stderr.decode("utf-8", errors="replace").strip()
    return AppError(
        message or "Touch ID authentication failed",
        code="native_auth_failed",
        retryable=False,
    )


def _read_fd_limited(fd: int, value: bytearray) -> None:
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


def _write_fd(fd: int, value: bytearray) -> None:
    view = memoryview(value)
    written = 0
    while written < len(view):
        written += os.write(fd, view[written:])
