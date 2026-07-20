"""Per-OS-user operator broker server."""

from __future__ import annotations

import ctypes
import ctypes.util
import os
from pathlib import Path
import secrets
import signal
import sys
import threading
from dataclasses import dataclass
from typing import Any

from ..command_capabilities import Capability
from ..errors import AppError
from ..log_ring import install_ring_logging, sanitize_traceback_text
from ..redaction import is_sensitive_key
from .protocol import PROTOCOL_VERSION, BrokerChannel, listen
from .project import canonical_project
from .policy import require_project_policy_binding
from .runner import run_cli_operation
from .service import OperatorService, _classify_argv, _wipe


MAX_CLIENT_THREADS = 64


class BrokerServer:
    def __init__(self) -> None:
        session_runtime = _login_session_runtime_root()
        logind_alive = _linux_logind_user_alive()
        if sys.platform.startswith("linux") and (
            logind_alive is False
            or (logind_alive is None and session_runtime is None)
        ):
            raise AppError(
                "a logout-bound operator broker is unavailable in this Linux session",
                code="operator_session_lifetime_unavailable",
                hint=(
                    "Use manual mode, or start the broker from a login session with "
                    "logind or an owner-only XDG_RUNTIME_DIR."
                ),
                retryable=False,
            )
        self.generation = secrets.token_hex(16)
        self.service = OperatorService(self.generation, run_cli_operation)
        self.listener = listen()
        self._stopped = threading.Event()
        self._close_lock = threading.Lock()
        self._listener_closed = False
        self._client_slots = threading.BoundedSemaphore(MAX_CLIENT_THREADS)
        self._session_runtime = session_runtime
        self._logind_user_observed = logind_alive is not None
        if (
            self._session_runtime is not None
            or self._logind_user_observed
        ):
            threading.Thread(
                target=self._monitor_login_session,
                name="operator-login-session",
                daemon=True,
            ).start()

    def serve_forever(self) -> None:
        while not self._stopped.is_set():
            try:
                channel = self.listener.accept()
            except AppError:
                continue
            except OSError:
                if self._stopped.is_set():
                    break
                continue
            if not self._client_slots.acquire(blocking=False):
                try:
                    with channel:
                        channel.send_json(
                            _error_response(
                                AppError(
                                    "the operator broker has too many connected clients",
                                    code="operator_client_limit",
                                    hint="Close an idle client or retry after an active request finishes.",
                                    details={"limit": MAX_CLIENT_THREADS},
                                    retryable=True,
                                )
                            )
                        )
                except (OSError, EOFError):
                    pass
                continue
            threading.Thread(
                target=self._serve_admitted_channel,
                args=(channel,),
                name="operator-client",
                daemon=True,
            ).start()

    def _serve_admitted_channel(self, channel: BrokerChannel) -> None:
        try:
            self._serve_channel(channel)
        finally:
            self._client_slots.release()

    def request_stop(self) -> None:
        """Stop admission without re-entering full cleanup from a signal."""

        self._stopped.set()
        # Python signal handlers run on the main thread and may interrupt an
        # in-progress close on that same thread. A blocking acquire would then
        # deadlock; the active close already owns listener shutdown.
        if not self._close_lock.acquire(blocking=False):
            return
        try:
            if self._listener_closed:
                return
            try:
                self.listener.close()
            except Exception:
                # Full close in main's finally block retries and reports the
                # listener failure while still attempting service cleanup.
                return
            self._listener_closed = True
        finally:
            self._close_lock.release()

    def close(self) -> None:
        with self._close_lock:
            self._stopped.set()
            first_error: BaseException | None = None
            if not self._listener_closed:
                try:
                    self.listener.close()
                except BaseException as exc:
                    first_error = exc
                else:
                    self._listener_closed = True
            try:
                # OperatorService.close() deliberately retries transient owner
                # release failures. Keep delegating after the listener has
                # stopped so a later close can complete that cleanup.
                self.service.close()
            except BaseException as exc:
                if first_error is None:
                    first_error = exc
            if first_error is not None:
                raise first_error

    def _monitor_login_session(self) -> None:
        while not self._stopped.wait(2.0):
            logind_alive = (
                _linux_logind_user_alive()
                if self._logind_user_observed
                else None
            )
            if not _linux_session_lifetime_is_valid(
                self._session_runtime,
                logind_observed=self._logind_user_observed,
                logind_alive=logind_alive,
            ):
                self.close()
                return

    def _serve_channel(self, channel: BrokerChannel) -> None:
        with channel:
            try:
                request = channel.receive_json()
                self._require_version(request)
                response = self._handle(channel, request)
            except EOFError:
                return
            except AppError as exc:
                response = _error_response(exc)
            except Exception:
                response = _error_response(
                    AppError(
                        "the operator broker encountered an internal error",
                        code="operator_internal_error",
                        retryable=True,
                    )
                )
            channel.send_json(response)

    def _handle(
        self,
        channel: BrokerChannel,
        request: dict[str, Any],
    ) -> dict[str, Any]:
        action = request.get("action")
        if action == "ping":
            return _ok({"broker": "running", "generation": self.generation})
        if action == "status":
            data_root = request.get("data_root")
            return _ok(
                self.service.status(
                    _canonical_data_root(data_root)
                    if isinstance(data_root, str)
                    else None
                )
            )
        if action == "unlock":
            data_root = _canonical_data_root(_required_string(request, "data_root"))
            if request.get("authentication_method", "password") != "password":
                raise AppError(
                    "password unlock cannot claim another authentication method",
                    code="operator_invalid_authentication_method",
                    retryable=False,
                )
            challenge = secrets.token_hex(24)
            channel.send_json(
                {
                    "ok": True,
                    "continue": "secret",
                    "label": "database_passphrase",
                    "challenge": challenge,
                }
            )
            passphrase = channel.receive_secret(challenge)
            try:
                duration, capability = _lease_request_args(request)
                return _ok(
                    self.service.unlock(
                        data_root,
                        passphrase,
                        duration_seconds=duration,
                        capability=capability,
                        authentication_method="password",
                    )
                )
            finally:
                _wipe(passphrase)
        if action == "unlock_touch_id":
            from .native_auth import broker_touch_id_passphrase

            data_root = _canonical_data_root(_required_string(request, "data_root"))
            binding = require_project_policy_binding(data_root)
            duration, capability = _lease_request_args(request)
            passphrase = broker_touch_id_passphrase(data_root)
            try:
                return _ok(
                    self.service.unlock(
                        data_root,
                        passphrase,
                        duration_seconds=duration,
                        capability=capability,
                        authentication_method="touch_id",
                        expected_database_identity=binding.database_identity,
                    )
                )
            finally:
                _wipe(passphrase)
        if action == "lock":
            return _ok(
                self.service.lock(
                    _canonical_data_root(_required_string(request, "data_root"))
                )
            )
        if action == "submit":
            return self._handle_submit(channel, request)
        if action == "operation_status":
            return _ok(
                self.service.operation_status(
                    _required_string(request, "operation_id"),
                    include_output=bool(request.get("include_output", True)),
                )
            )
        if action == "operation_cancel":
            return _ok(
                self.service.cancel(_required_string(request, "operation_id"))
            )
        if action == "set_mode":
            data_root = _canonical_data_root(_required_string(request, "data_root"))
            mode = _required_string(request, "mode")
            challenge = secrets.token_hex(24)
            channel.send_json(
                {
                    "ok": True,
                    "continue": "secret",
                    "label": "fresh_admin_auth",
                    "challenge": challenge,
                }
            )
            authentication = channel.receive_secret(challenge)
            try:
                return _ok(
                    self.service.set_mode_authenticated(
                        data_root,
                        authentication,
                        mode,
                    )
                )
            finally:
                _wipe(authentication)
        if action == "touch_id_configure":
            data_root = _canonical_data_root(_required_string(request, "data_root"))
            configured = request.get("configured")
            if not isinstance(configured, bool):
                raise AppError(
                    "Touch ID configuration requires a boolean state",
                    code="operator_protocol_error",
                    retryable=False,
                )
            challenge = secrets.token_hex(24)
            channel.send_json(
                {
                    "ok": True,
                    "continue": "secret",
                    "label": "fresh_native_auth",
                    "challenge": challenge,
                }
            )
            authentication = channel.receive_secret(challenge)
            try:
                return _ok(
                    self.service.configure_touch_id_authenticated(
                        data_root,
                        authentication,
                        configured=configured,
                    )
                )
            finally:
                _wipe(authentication)
        raise AppError(
            "unknown operator broker action",
            code="operator_protocol_error",
            retryable=False,
        )

    def _handle_submit(
        self,
        channel: BrokerChannel,
        request: dict[str, Any],
    ) -> dict[str, Any]:
        data_root = _canonical_data_root(_required_string(request, "data_root"))
        operation_id = _required_string(request, "operation_id")
        argv = request.get("argv")
        labels = request.get("secret_labels", [])
        if not isinstance(argv, list) or not all(isinstance(value, str) for value in argv):
            raise AppError("invalid broker argv", code="operator_protocol_error")
        if (
            not isinstance(labels, list)
            or len(labels) > 32
            or not all(isinstance(value, str) and value.startswith("broker-secret-") for value in labels)
            or len(set(labels)) != len(labels)
        ):
            raise AppError("invalid broker secret labels", code="operator_protocol_error")
        command_path, capability = _classify_argv(argv)
        admin_command_label = None
        if command_path == "secrets.remember-unlock" and "--passphrase-fd" not in argv:
            admin_command_label = f"broker-secret-{secrets.token_hex(16)}"
            argv = [*argv, "--passphrase-fd", admin_command_label]
        challenges = {label: secrets.token_hex(24) for label in labels}
        admin_challenge = secrets.token_hex(24) if capability is Capability.ADMIN else None
        if challenges or admin_challenge is not None:
            channel.send_json(
                {
                    "ok": True,
                    "continue": "secrets",
                    "challenges": challenges,
                    "admin_challenge": admin_challenge,
                }
            )
        secret_arguments: dict[str, bytearray] = {}
        admin_auth: bytearray | None = None
        admin_authorization = None
        try:
            # JSON encoding sorts mapping keys for deterministic wire output.
            # Receive in that same explicit order so multiple secret frames
            # cannot be assigned to a different challenge after serialization.
            for label in sorted(challenges):
                challenge = challenges[label]
                secret_arguments[label] = channel.receive_secret(challenge)
            if admin_challenge is not None:
                admin_auth = channel.receive_secret(admin_challenge)
                admin_authorization = self.service.verify_admin(
                    data_root,
                    admin_auth,
                )
                if admin_command_label is not None:
                    secret_arguments[admin_command_label] = bytearray(admin_auth)
            return _ok(
                self.service.submit(
                    data_root,
                    argv,
                    operation_id=operation_id,
                    secret_arguments=secret_arguments,
                    admin_authorization=admin_authorization,
                )
            )
        except Exception:
            for value in secret_arguments.values():
                _wipe(value)
            raise
        finally:
            if admin_auth is not None:
                _wipe(admin_auth)

    @staticmethod
    def _require_version(request: dict[str, Any]) -> None:
        if request.get("version") != PROTOCOL_VERSION:
            raise AppError(
                "operator protocol version mismatch",
                code="operator_protocol_version_mismatch",
                details={"supported": PROTOCOL_VERSION},
                retryable=False,
            )


def _required_string(payload: dict[str, Any], key: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or not value:
        raise AppError(
            f"operator request requires {key}",
            code="operator_protocol_error",
            retryable=False,
        )
    return value


def _canonical_data_root(data_root: str) -> str:
    return str(canonical_project(data_root).database.parent)


def _lease_request_args(
    request: dict[str, Any],
) -> tuple[int | None, Capability]:
    duration = request.get("duration_seconds")
    if duration is not None and not isinstance(duration, int):
        raise AppError(
            "invalid operator duration",
            code="operator_invalid_duration",
            retryable=False,
        )
    try:
        capability = Capability(
            request.get("capability", Capability.ACCOUNTING_DECISIONS.value)
        )
    except (TypeError, ValueError) as exc:
        raise AppError(
            "invalid operator lease capability",
            code="operator_invalid_lease_capability",
            retryable=False,
        ) from exc
    return duration, capability


def _ok(data: dict[str, object]) -> dict[str, Any]:
    return {"ok": True, "data": data}


def _error_response(exc: AppError) -> dict[str, Any]:
    return {
        "ok": False,
        "error": {
            "code": exc.code,
            "message": sanitize_traceback_text(str(exc)),
            "hint": sanitize_traceback_text(exc.hint) if exc.hint else None,
            "details": _public_safe_details(exc.details),
            "retryable": bool(exc.retryable),
        },
    }


def _public_safe_details(value: object) -> object:
    if isinstance(value, dict):
        safe: dict[str, object] = {}
        for key, item in value.items():
            key_text = str(key)
            if key_text.lower() in {"path", "database", "data_root"}:
                continue
            safe[key_text] = (
                "[redacted]" if is_sensitive_key(key_text) else _public_safe_details(item)
            )
        return safe
    if isinstance(value, list):
        return [_public_safe_details(item) for item in value]
    if isinstance(value, str):
        return sanitize_traceback_text(value)
    if isinstance(value, (int, float, bool)) or value is None:
        return value
    return sanitize_traceback_text(str(value))


@dataclass(frozen=True)
class _LoginSessionRuntime:
    root: Path
    device: int
    inode: int


def _login_session_runtime_root() -> _LoginSessionRuntime | None:
    """Return Linux's per-login-user runtime root when one is available."""

    if not sys.platform.startswith("linux"):
        return None
    configured = os.environ.get("XDG_RUNTIME_DIR")
    if not configured:
        return None
    try:
        root = Path(configured).resolve(strict=True)
        info = root.stat()
    except OSError:
        return None
    runtime = _LoginSessionRuntime(root, info.st_dev, info.st_ino)
    return runtime if _login_session_runtime_is_valid(runtime) else None


def _login_session_runtime_is_valid(runtime: _LoginSessionRuntime | None) -> bool:
    if runtime is None:
        return False
    try:
        info = runtime.root.stat()
    except OSError:
        return False
    return (
        (not hasattr(os, "getuid") or info.st_uid == os.getuid())
        and info.st_dev == runtime.device
        and info.st_ino == runtime.inode
    )


def _linux_session_lifetime_is_valid(
    runtime: _LoginSessionRuntime | None,
    *,
    logind_observed: bool,
    logind_alive: bool | None,
) -> bool:
    if runtime is not None and not _login_session_runtime_is_valid(runtime):
        return False
    if logind_alive is False:
        return False
    if runtime is not None:
        return True
    return logind_observed and logind_alive is True


def _linux_logind_user_alive() -> bool | None:
    """Return whether logind still sees an interactive login for this OS user."""

    if not sys.platform.startswith("linux") or not hasattr(os, "getuid"):
        return None
    systemd = _load_systemd()
    if systemd is None:
        return None
    get_state = systemd.sd_uid_get_state
    get_state.argtypes = [ctypes.c_uint, ctypes.POINTER(ctypes.c_void_p)]
    get_state.restype = ctypes.c_int
    pointer = ctypes.c_void_p()
    result = get_state(os.getuid(), ctypes.byref(pointer))
    if result < 0 or not pointer.value:
        # A missing logind user record (for example on a non-systemd Linux
        # host that still ships libsystemd) is not evidence of logout. The
        # pinned XDG runtime-directory guard is the only supported fallback.
        return None
    try:
        state = ctypes.string_at(pointer).decode("utf-8")
    finally:
        libc = ctypes.CDLL(None)
        libc.free.argtypes = [ctypes.c_void_p]
        libc.free(pointer)
    return _logind_user_state_is_alive(state)


def _logind_user_state_is_alive(state: str) -> bool:
    # `online` is a valid logged-in but non-foreground user. `closing`,
    # `lingering`, and `offline` have no live interactive login and must drop
    # the in-memory authorization even if a user manager keeps running.
    return state in {"online", "active"}


def _load_systemd() -> ctypes.CDLL | None:
    library = ctypes.util.find_library("systemd")
    if not library:
        return None
    try:
        return ctypes.CDLL(library)
    except OSError:
        return None


def main() -> int:
    install_ring_logging()
    server = BrokerServer()

    def stop(_signum: int, _frame: object) -> None:
        server.request_stop()

    if os.name != "nt":
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
    try:
        server.serve_forever()
    finally:
        server.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
