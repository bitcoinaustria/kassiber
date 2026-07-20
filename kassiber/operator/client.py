"""Terminal broker client, launch rendezvous, and secret argument transport."""

from __future__ import annotations

import os
import secrets
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import BinaryIO

from ..errors import AppError
from .protocol import PROTOCOL_VERSION, BrokerChannel, connect
from .service import _wipe
from .launcher import broker_server_command


TERMINAL_OPERATION_STATES = frozenset(
    {"completed", "failed", "cancelled", "result_unknown"}
)
MAX_CLIENT_SECRET_BYTES = 16 * 1024


@dataclass
class PreparedArguments:
    argv: list[str]
    secrets: dict[str, bytearray]


class BrokerClient:
    def ensure_running(self) -> dict[str, object]:
        try:
            return self.ping()
        except (OSError, EOFError, AppError):
            pass
        environment = os.environ.copy()
        environment.pop("KASSIBER_OPERATOR_DIRECT", None)
        popen_args: dict[str, object] = {
            "stdin": subprocess.DEVNULL,
            "stdout": subprocess.DEVNULL,
            "stderr": subprocess.DEVNULL,
            "env": environment,
            "close_fds": True,
        }
        if os.name == "nt":
            popen_args["creationflags"] = (
                subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS
            )
        else:
            popen_args["start_new_session"] = True
        subprocess.Popen(broker_server_command(), **popen_args)
        deadline = time.monotonic() + 5.0
        last_error: Exception | None = None
        while time.monotonic() < deadline:
            try:
                return self.ping()
            except (OSError, EOFError, AppError) as exc:
                last_error = exc
                time.sleep(0.05)
        raise AppError(
            "the operator broker did not become ready",
            code="operator_broker_start_failed",
            hint="Run the command again with --debug to inspect local startup failures.",
            details={"reason": last_error.__class__.__name__ if last_error else "unknown"},
            retryable=True,
        )

    def ping(self) -> dict[str, object]:
        with connect(timeout=1.0, io_timeout=1.0) as channel:
            channel.send_json({"version": PROTOCOL_VERSION, "action": "ping"})
            return self._receive_data(channel)

    def status(self, data_root: str | None) -> dict[str, object]:
        try:
            return self._simple_request("status", data_root=data_root)
        except (OSError, EOFError):
            return {"broker": "stopped", "lease": "locked"}

    def unlock(
        self,
        data_root: str,
        passphrase: bytearray,
        *,
        duration_seconds: int | None,
        capability: str,
        authentication_method: str,
    ) -> dict[str, object]:
        self.ensure_running()
        with connect() as channel:
            channel.send_json(
                {
                    "version": PROTOCOL_VERSION,
                    "action": "unlock",
                    "data_root": data_root,
                    "duration_seconds": duration_seconds,
                    "capability": capability,
                    "authentication_method": authentication_method,
                }
            )
            continuation = self._receive(channel)
            challenge = continuation.get("challenge")
            if continuation.get("continue") != "secret" or not isinstance(challenge, str):
                raise AppError("invalid broker unlock challenge", code="operator_protocol_error")
            channel.send_secret(challenge, passphrase)
            return self._receive_data(channel)

    def unlock_touch_id(
        self,
        data_root: str,
        *,
        duration_seconds: int | None,
        capability: str,
    ) -> dict[str, object]:
        self.ensure_running()
        with connect() as channel:
            channel.send_json(
                {
                    "version": PROTOCOL_VERSION,
                    "action": "unlock_touch_id",
                    "data_root": data_root,
                    "duration_seconds": duration_seconds,
                    "capability": capability,
                }
            )
            return self._receive_data(channel)

    def lock(self, data_root: str) -> dict[str, object]:
        try:
            return self._simple_request("lock", data_root=data_root)
        except (OSError, EOFError):
            return {"broker": "stopped", "locked": True, "lease_existed": False}

    def set_mode(
        self,
        data_root: str,
        mode: str,
        authentication: bytearray,
    ) -> dict[str, object]:
        self.ensure_running()
        with connect() as channel:
            channel.send_json(
                {
                    "version": PROTOCOL_VERSION,
                    "action": "set_mode",
                    "data_root": data_root,
                    "mode": mode,
                }
            )
            continuation = self._receive(channel)
            challenge = continuation.get("challenge")
            if continuation.get("continue") != "secret" or not isinstance(challenge, str):
                raise AppError("invalid broker mode challenge", code="operator_protocol_error")
            channel.send_secret(challenge, authentication)
            return self._receive_data(channel)

    def configure_touch_id(
        self,
        data_root: str,
        authentication: bytearray,
        *,
        configured: bool,
    ) -> dict[str, object]:
        self.ensure_running()
        with connect() as channel:
            channel.send_json(
                {
                    "version": PROTOCOL_VERSION,
                    "action": "touch_id_configure",
                    "data_root": data_root,
                    "configured": configured,
                }
            )
            continuation = self._receive(channel)
            challenge = continuation.get("challenge")
            if continuation.get("continue") != "secret" or not isinstance(challenge, str):
                raise AppError(
                    "invalid broker native-auth challenge",
                    code="operator_protocol_error",
                )
            channel.send_secret(challenge, authentication)
            return self._receive_data(channel)

    def submit(
        self,
        data_root: str,
        prepared: PreparedArguments,
        *,
        admin_authentication: bytearray | None,
    ) -> dict[str, object]:
        broker = self.ensure_running()
        generation = broker.get("generation")
        if not isinstance(generation, str):
            raise AppError("broker generation is unavailable", code="operator_protocol_error")
        operation_id = f"{generation}.client.{secrets.token_hex(16)}"
        try:
            return self._submit_once(
                data_root,
                prepared,
                operation_id=operation_id,
                admin_authentication=admin_authentication,
            )
        except (OSError, EOFError):
            try:
                return self._submit_once(
                    data_root,
                    prepared,
                    operation_id=operation_id,
                    admin_authentication=admin_authentication,
                )
            except (OSError, EOFError, AppError) as retry_exc:
                try:
                    status = self.operation_status(operation_id)
                except (OSError, EOFError, AppError):
                    status = {
                        "operation_id": operation_id,
                        "state": "result_unknown",
                        "reason": "broker_status_unavailable",
                    }
                if status.get("state") in {
                    "queued",
                    "running",
                    "completed",
                    "failed",
                    "cancelled",
                }:
                    return status
                reason = status.get("reason")
                if not isinstance(reason, str):
                    reason = "submission_acknowledgement_lost"
                raise AppError(
                    "the broker could not prove the result of the submitted operation",
                    code="operator_submission_result_unknown",
                    hint="Reconcile project state before retrying the operation.",
                    details={
                        "operation_id": operation_id,
                        "state": "result_unknown",
                        "reason": reason,
                    },
                    retryable=False,
                ) from retry_exc

    def _submit_once(
        self,
        data_root: str,
        prepared: PreparedArguments,
        *,
        operation_id: str,
        admin_authentication: bytearray | None,
    ) -> dict[str, object]:
        with connect() as channel:
            channel.send_json(
                {
                    "version": PROTOCOL_VERSION,
                    "action": "submit",
                    "data_root": data_root,
                    "operation_id": operation_id,
                    "argv": prepared.argv,
                    "secret_labels": list(prepared.secrets),
                }
            )
            response = self._receive(channel)
            if response.get("continue") == "secrets":
                challenges = response.get("challenges")
                if not isinstance(challenges, dict):
                    raise AppError("invalid broker secret challenge", code="operator_protocol_error")
                for label, challenge in challenges.items():
                    secret = prepared.secrets.get(str(label))
                    if secret is None or not isinstance(challenge, str):
                        raise AppError("invalid broker secret challenge", code="operator_protocol_error")
                    channel.send_secret(challenge, secret)
                admin_challenge = response.get("admin_challenge")
                if admin_challenge is not None:
                    if admin_authentication is None:
                        raise AppError(
                            "fresh authentication is required for this admin command",
                            code="operator_admin_auth_required",
                            retryable=False,
                        )
                    if not isinstance(admin_challenge, str):
                        raise AppError("invalid broker admin challenge", code="operator_protocol_error")
                    channel.send_secret(admin_challenge, admin_authentication)
                return self._receive_data(channel)
            return self._data_from_response(response)

    def operation_status(self, operation_id: str) -> dict[str, object]:
        try:
            return self._simple_request(
                "operation_status",
                operation_id=operation_id,
                include_output=True,
            )
        except (OSError, EOFError):
            return {
                "operation_id": operation_id,
                "state": "result_unknown",
                "reason": "broker_unreachable",
                "hint": "Reconcile project state before retrying the operation.",
            }

    def cancel(self, operation_id: str) -> dict[str, object]:
        return self._simple_request("operation_cancel", operation_id=operation_id)

    def wait(self, operation_id: str) -> dict[str, object]:
        delay = 0.05
        while True:
            status = self.operation_status(operation_id)
            if status.get("state") in TERMINAL_OPERATION_STATES:
                return status
            time.sleep(delay)
            delay = min(0.5, delay * 1.4)

    def _simple_request(self, action: str, **fields: object) -> dict[str, object]:
        with connect(io_timeout=2.0) as channel:
            channel.send_json(
                {"version": PROTOCOL_VERSION, "action": action, **fields}
            )
            return self._receive_data(channel)

    def _receive_data(self, channel: BrokerChannel) -> dict[str, object]:
        return self._data_from_response(self._receive(channel))

    @staticmethod
    def _receive(channel: BrokerChannel) -> dict[str, object]:
        response = channel.receive_json()
        if response.get("ok") is False:
            error = response.get("error")
            if not isinstance(error, dict):
                raise AppError("invalid broker error response", code="operator_protocol_error")
            raise AppError(
                str(error.get("message") or "operator broker error"),
                code=str(error.get("code") or "operator_error"),
                details=error.get("details"),
                hint=error.get("hint") if isinstance(error.get("hint"), str) else None,
                retryable=bool(error.get("retryable")),
            )
        return response

    @staticmethod
    def _data_from_response(response: dict[str, object]) -> dict[str, object]:
        data = response.get("data")
        if not isinstance(data, dict):
            raise AppError("invalid broker response", code="operator_protocol_error")
        return data


def prepare_arguments(
    argv: list[str],
    *,
    stdin: BinaryIO | None = None,
) -> PreparedArguments:
    """Replace secret fd/stdin inputs with opaque labels for binary framing."""

    prepared: list[str] = []
    secret_values: dict[str, bytearray] = {}
    stdin = stdin or sys.stdin.buffer
    index = 0
    stdin_consumed = False
    try:
        while index < len(argv):
            token = argv[index]
            if token.startswith("--") and token.endswith("-fd"):
                if index + 1 >= len(argv):
                    raise AppError("secret fd flag requires a value", code="operator_invalid_command")
                label = f"broker-secret-{secrets.token_hex(16)}"
                secret_values[label] = _read_secret_fd(int(argv[index + 1]))
                prepared.extend((token, label))
                index += 2
                continue
            if token.startswith("--") and token.endswith("-stdin"):
                if stdin_consumed:
                    raise AppError(
                        "only one stdin secret can be brokered per command",
                        code="multiple_stdin_secrets",
                    )
                label = f"broker-secret-{secrets.token_hex(16)}"
                secret_values[label] = _read_limited(stdin)
                prepared.extend((token[: -len("stdin")] + "fd", label))
                stdin_consumed = True
                index += 1
                continue
            prepared.append(token)
            index += 1
    except Exception:
        for secret in secret_values.values():
            _wipe(secret)
        raise
    return PreparedArguments(prepared, secret_values)


def wipe_prepared(prepared: PreparedArguments) -> None:
    for secret in prepared.secrets.values():
        _wipe(secret)
    prepared.secrets.clear()


def parse_duration(value: str) -> int:
    text = value.strip().lower()
    multipliers = {"m": 60, "h": 3600, "d": 86400}
    if len(text) < 2 or text[-1] not in multipliers:
        raise AppError(
            "duration must use m, h, or d (for example 30m or 8h)",
            code="operator_invalid_duration",
            retryable=False,
        )
    try:
        amount = int(text[:-1])
    except ValueError as exc:
        raise AppError("invalid operator duration", code="operator_invalid_duration") from exc
    seconds = amount * multipliers[text[-1]]
    if seconds < 60:
        raise AppError(
            "operator duration must be at least 1 minute",
            code="operator_invalid_duration",
            retryable=False,
        )
    return seconds


def _read_secret_fd(fd: int) -> bytearray:
    try:
        with os.fdopen(fd, "rb", closefd=True) as handle:
            return _read_limited(handle)
    except OSError as exc:
        raise AppError(
            "could not read brokered secret fd",
            code="secret_input_error",
            retryable=False,
        ) from exc


def _read_limited(handle: BinaryIO) -> bytearray:
    value = bytearray()
    while True:
        chunk = handle.read(min(4096, MAX_CLIENT_SECRET_BYTES + 1 - len(value)))
        if not chunk:
            break
        value.extend(chunk)
        if len(value) > MAX_CLIENT_SECRET_BYTES:
            _wipe(value)
            raise AppError(
                "brokered secret exceeds the 16 KiB limit",
                code="operator_secret_too_large",
                retryable=False,
            )
    while value.endswith((b"\n", b"\r")):
        value.pop()
    if not value:
        raise AppError("brokered secret is empty", code="secret_input_error")
    return value
