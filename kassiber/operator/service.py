"""In-memory leases, per-project queues, and recoverable operation state."""

from __future__ import annotations

import logging
import secrets
import subprocess
import threading
import time
from collections import OrderedDict, deque
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Callable

from ..command_capabilities import Capability, capability_allows, cli_capability
from ..core.repo import current_context_snapshot
from ..core.runtime import resolve_runtime_paths
from ..db import database_instance_id, open_db, resolve_config_root
from ..errors import AppError
from ..log_ring import sanitize_traceback_text
from ..redaction import redact_secret_text
from ..secrets.auth_backoff import AuthAttemptBackoff, AUTH_BACKOFF_FILENAME
from ..time_utils import now_iso
from .modes import set_unlock_mode
from .project import (
    CanonicalProject,
    ProjectOwnerLease,
    acquire_project_ownership,
    canonical_project,
)


MAX_QUEUED_OPERATIONS = 64
MAX_RETAINED_RESULTS = 256
MAX_RETAINED_OPERATION_TOMBSTONES = 1024
ADMIN_AUTH_TTL_SECONDS = 60.0
_LOGGER = logging.getLogger("kassiber.operator")
OperationRunner = Callable[["Operation", bytearray], "OperationResult"]


@dataclass(frozen=True)
class OperationResult:
    exit_code: int
    stdout: str
    stderr: str


@dataclass
class Operation:
    id: str
    generation: str
    project_id: str
    project_identity: str
    database_identity: str
    data_root: str
    argv: list[str]
    command_path: str
    capability: Capability
    secret_arguments: dict[str, bytearray]
    admin_authorized_until_monotonic: float | None = None
    state: str = "queued"
    submitted_at: str = field(default_factory=now_iso)
    started_at: str | None = None
    finished_at: str | None = None
    result: OperationResult | None = None
    cancellation_requested: bool = False
    process: subprocess.Popen[bytes] | None = field(default=None, repr=False)
    owner_handle_tokens: tuple[int, ...] = field(default=(), repr=False)
    changed: threading.Condition = field(
        default_factory=threading.Condition,
        repr=False,
    )

    def public_status(self, *, include_output: bool = False) -> dict[str, object]:
        payload: dict[str, object] = {
            "operation_id": self.id,
            "project": self.project_id,
            "command": self.command_path,
            "capability": self.capability.value,
            "state": self.state,
            "submitted_at": self.submitted_at,
            "started_at": self.started_at,
            "finished_at": self.finished_at,
        }
        if self.result is not None:
            payload["exit_code"] = self.result.exit_code
            if include_output:
                payload["stdout"] = self.result.stdout
                payload["stderr"] = self.result.stderr
        return payload


@dataclass
class ProjectLease:
    data_root: str
    project: CanonicalProject
    database_identity: str
    passphrase: bytearray
    capability: Capability
    owner: ProjectOwnerLease
    unlocked_at: str
    expires_at_monotonic: float | None
    duration_seconds: int | None
    authentication_method: str
    expires_at: str | None
    workspace: str | None = None
    profile: str | None = None
    revoked: bool = False
    running_operations: int = 0

    def expired(self) -> bool:
        return (
            self.expires_at_monotonic is not None
            and time.monotonic() >= self.expires_at_monotonic
        )


class ProjectWorker:
    def __init__(
        self,
        service: OperatorService,
        project_identity: str,
        project_public_id: str,
        runner: OperationRunner,
    ) -> None:
        self._service = service
        self.project_identity = project_identity
        self.project_id = project_public_id
        self._runner = runner
        self._queue: deque[Operation] = deque()
        self._condition = threading.Condition()
        self._stopping = False
        self._thread = threading.Thread(
            target=self._run,
            name=f"operator-project-{project_public_id}",
            daemon=True,
        )
        self._thread.start()

    @property
    def queued(self) -> int:
        with self._condition:
            return len(self._queue)

    def submit(self, operation: Operation) -> None:
        with self._condition:
            if len(self._queue) >= MAX_QUEUED_OPERATIONS:
                raise AppError(
                    "the project operation queue is full",
                    code="operator_queue_full",
                    details={
                        "project": self.project_id,
                        "limit": MAX_QUEUED_OPERATIONS,
                    },
                    retryable=True,
                )
            self._queue.append(operation)
            self._condition.notify()

    def remove(self, operation: Operation) -> bool:
        with self._condition:
            try:
                self._queue.remove(operation)
            except ValueError:
                return False
            return True

    def drain(self) -> list[Operation]:
        with self._condition:
            operations = list(self._queue)
            self._queue.clear()
            return operations

    def stop(self) -> None:
        with self._condition:
            self._stopping = True
            self._condition.notify_all()

    def _run(self) -> None:
        while True:
            with self._condition:
                while not self._queue and not self._stopping:
                    self._condition.wait()
                if not self._queue:
                    return
                operation = self._queue.popleft()
            self._run_one(operation)

    def _run_one(self, operation: Operation) -> None:
        with self._service._lock:
            lease = self._service._leases.get(self.project_identity)
            if operation.cancellation_requested:
                return
            if lease is None or lease.revoked or lease.expired():
                self._service._finish_operation_locked(
                    operation,
                    "cancelled",
                    OperationResult(
                        1,
                        "",
                        "operator lease ended before the operation started\n",
                    ),
                )
                if lease is not None:
                    lease.revoked = True
                    if lease.running_operations == 0:
                        self._service._drop_lease_locked(self.project_identity)
                return
            if (
                operation.capability is not Capability.ADMIN
                and not capability_allows(lease.capability, operation.capability)
            ):
                self._service._finish_operation_locked(
                    operation,
                    "cancelled",
                    OperationResult(
                        1,
                        "",
                        "operator lease capability changed before dispatch\n",
                    ),
                )
                return
            try:
                current_project = canonical_project(operation.data_root)
            except Exception:
                current_project = None
            if (
                current_project is None
                or current_project.identity != operation.project_identity
            ):
                self._service._finish_operation_locked(
                    operation,
                    "cancelled",
                    OperationResult(
                        1,
                        "",
                        "operator project changed before the operation started\n",
                    ),
                )
                return
            if operation.capability is Capability.ADMIN and (
                operation.admin_authorized_until_monotonic is None
                or time.monotonic() > operation.admin_authorized_until_monotonic
            ):
                self._service._finish_operation_locked(
                    operation,
                    "cancelled",
                    OperationResult(
                        1,
                        "",
                        "fresh admin authentication expired before dispatch\n",
                    ),
                )
                return
            try:
                inherited_owner = lease.owner.duplicate_for_child()
            except Exception as exc:
                self._service._finish_operation_locked(
                    operation,
                    "result_unknown",
                    OperationResult(
                        1,
                        "",
                        sanitize_traceback_text(
                            f"operator ownership inheritance failed: {exc}\n"
                        ),
                    ),
                )
                return
            lease.running_operations += 1
            operation.owner_handle_tokens = inherited_owner.tokens
            operation.state = "running"
            operation.started_at = now_iso()
            _LOGGER.info(
                "operator operation dispatched",
                extra={
                    "kb_fields": {
                        "project": self.project_id,
                        "command": operation.command_path,
                        "state": "running",
                    }
                },
            )
            with operation.changed:
                operation.changed.notify_all()
            passphrase = lease.passphrase
        runner_crashed = False
        try:
            result = self._runner(operation, passphrase)
        except Exception as exc:
            runner_crashed = True
            result = OperationResult(
                1,
                "",
                sanitize_traceback_text(f"operator child failed: {exc}\n"),
            )
        finally:
            operation.owner_handle_tokens = ()
            inherited_owner.close()
        result = OperationResult(
            result.exit_code,
            result.stdout,
            redact_secret_text(result.stderr),
        )
        with self._service._lock:
            current_lease = self._service._leases.get(self.project_identity)
            if current_lease is not None:
                current_lease.running_operations = max(
                    0, current_lease.running_operations - 1
                )
            if runner_crashed or result.exit_code < 0:
                state = "result_unknown"
            elif result.exit_code != 0 and operation.capability is not Capability.READ:
                state = "result_unknown"
            else:
                state = "completed" if result.exit_code == 0 else "failed"
            self._service._finish_operation_locked(operation, state, result)
            if (
                operation.command_path == "context.set"
                and state == "completed"
                and current_lease is not None
                and not current_lease.revoked
            ):
                self._service._refresh_scope_locked(current_lease)
            _LOGGER.info(
                "operator operation finished",
                extra={
                    "kb_fields": {
                        "project": self.project_id,
                        "command": operation.command_path,
                        "state": state,
                    }
                },
            )
            if operation.command_path == "secrets.change-passphrase":
                self._service._revoke_lease_locked(
                    self.project_identity,
                    reason="database passphrase rotation ended the prior lease",
                )
            if (
                current_lease is not None
                and (current_lease.revoked or current_lease.expired())
                and current_lease.running_operations == 0
            ):
                self._service._drop_lease_locked(self.project_identity)


class OperatorService:
    def __init__(self, generation: str, runner: OperationRunner) -> None:
        self.generation = generation
        self._runner = runner
        self._lock = threading.RLock()
        self._leases: dict[str, ProjectLease] = {}
        self._workers: dict[str, ProjectWorker] = {}
        self._operations: OrderedDict[str, Operation] = OrderedDict()
        self._operation_tombstones: OrderedDict[
            str, tuple[str, tuple[str, ...]]
        ] = OrderedDict()
        self._lease_aliases: dict[str, str] = {}
        self._auth_backoffs: dict[str, AuthAttemptBackoff] = {}
        self._sequence = 0
        self._closed = threading.Event()
        self._janitor = threading.Thread(
            target=self._expire_leases,
            name="operator-lease-expiry",
            daemon=True,
        )
        self._janitor.start()

    def unlock(
        self,
        data_root: str,
        passphrase: bytearray,
        *,
        duration_seconds: int | None,
        capability: Capability = Capability.ACCOUNTING_DECISIONS,
        authentication_method: str = "password",
    ) -> dict[str, object]:
        if capability not in {
            Capability.READ,
            Capability.OPERATOR,
            Capability.ACCOUNTING_DECISIONS,
        }:
            raise AppError(
                "admin is not a lease capability",
                code="operator_invalid_lease_capability",
                retryable=False,
            )
        if authentication_method not in {"password", "touch_id"}:
            raise AppError(
                "unsupported operator authentication method",
                code="operator_invalid_authentication_method",
                retryable=False,
            )
        if duration_seconds is not None and duration_seconds < 60:
            raise AppError(
                "operator lease duration must be at least 1 minute",
                code="operator_invalid_duration",
                retryable=False,
            )
        try:
            requested_expires_at = (
                (datetime.now(timezone.utc) + timedelta(seconds=duration_seconds))
                .replace(microsecond=0)
                .isoformat()
                .replace("+00:00", "Z")
                if duration_seconds is not None
                else None
            )
        except OverflowError as exc:
            raise AppError(
                "operator lease duration exceeds the timestamp range",
                code="operator_invalid_duration",
                retryable=False,
            ) from exc
        project = canonical_project(data_root)
        canonical_data_root = str(project.database.parent)
        alias = str(project.database)
        with self._lock:
            replaced_identity = self._lease_aliases.get(alias)
            if replaced_identity is not None and replaced_identity != project.identity:
                raise AppError(
                    "the database file changed while its prior operator lease is active",
                    code="operator_project_replaced",
                    hint="Lock the prior lease before unlocking the replacement database.",
                    retryable=False,
                )
            previous = self._leases.get(project.identity)
            if previous is not None and previous.running_operations > 0:
                raise AppError(
                    "the project still has a running operator operation",
                    code="operator_project_busy",
                    hint="Wait for the running operation to finish, then refresh the lease.",
                    details={"project": project.public_id},
                    retryable=True,
                )
            acquired_here = previous is None
            owner = previous.owner if previous is not None else acquire_project_ownership(
                    project,
                    owner_kind="broker",
                    generation=self.generation,
                )
            if previous is not None:
                owner.add_alias(project)
            backoff = self._auth_backoffs.setdefault(
                project.identity,
                AuthAttemptBackoff(
                    str(
                        resolve_config_root(canonical_data_root)
                        / AUTH_BACKOFF_FILENAME
                    )
                ),
            )
            try:
                backoff.check("operator_unlock")
                connection = open_db(
                    canonical_data_root,
                    passphrase=_decode_secret(passphrase),
                    require_existing_schema=True,
                )
                try:
                    if hasattr(connection, "execute"):
                        opened_database_identity = database_instance_id(connection)
                        context = current_context_snapshot(connection)
                    else:
                        opened_database_identity = project.identity
                        context = {}
                finally:
                    connection.close()
            except AppError as exc:
                if exc.code in {"unlock_failed", "passphrase_required"}:
                    backoff.record_failure()
                if acquired_here:
                    owner.release()
                raise
            except Exception:
                if acquired_here:
                    owner.release()
                raise
            else:
                backoff.record_success()
            try:
                set_unlock_mode(canonical_data_root, "brokered")
            except Exception:
                if acquired_here:
                    owner.release()
                raise
            stored = bytearray(passphrase)
            if previous is not None:
                _wipe(previous.passphrase)
            expires = (
                time.monotonic() + duration_seconds
                if duration_seconds is not None
                else None
            )
            self._leases[project.identity] = ProjectLease(
                data_root=canonical_data_root,
                project=project,
                database_identity=opened_database_identity,
                passphrase=stored,
                capability=capability,
                owner=owner,
                unlocked_at=now_iso(),
                expires_at_monotonic=expires,
                duration_seconds=duration_seconds,
                authentication_method=authentication_method,
                expires_at=requested_expires_at,
                workspace=(str(context.get("workspace_id")) or None)
                if context.get("workspace_id")
                else None,
                profile=(str(context.get("profile_id")) or None)
                if context.get("profile_id")
                else None,
            )
            self._lease_aliases[alias] = project.identity
            self._workers.setdefault(
                project.identity,
                ProjectWorker(
                    self,
                    project.identity,
                    project.public_id,
                    self._runner,
                ),
            )
        _LOGGER.info(
            "operator lease unlocked",
            extra={
                "kb_fields": {
                    "project": project.public_id,
                    "capability": capability.value,
                    "authentication_method": authentication_method,
                }
            },
        )
        return self.status(canonical_data_root)

    def verify_admin(self, data_root: str, secret: bytearray) -> None:
        project = canonical_project(data_root)
        canonical_data_root = str(project.database.parent)
        with self._lock:
            self._require_lease_locked(project)
        self.authenticate_database(
            canonical_data_root,
            secret,
            scope="operator_admin",
        )

    def authenticate_database(
        self,
        data_root: str,
        secret: bytearray,
        *,
        scope: str,
        require_lease: bool = True,
        continuation: Callable[[], object] | None = None,
    ) -> object | None:
        project = canonical_project(data_root)
        canonical_data_root = str(project.database.parent)
        with self._lock:
            temporary_owner: ProjectOwnerLease | None = None
            if require_lease:
                self._require_lease_locked(project)
            else:
                existing = self._leases.get(project.identity)
                if existing is not None and (
                    existing.revoked or existing.expired()
                ):
                    if existing.running_operations:
                        raise AppError(
                            "the project still has a running operator operation",
                            code="operator_project_busy",
                            retryable=True,
                        )
                    self._drop_lease_locked(project.identity)
                    existing = None
                if existing is None:
                    temporary_owner = acquire_project_ownership(
                        project,
                        owner_kind="broker",
                        generation=self.generation,
                    )
            try:
                backoff = self._auth_backoffs.setdefault(
                    project.identity,
                    AuthAttemptBackoff(
                        str(
                            resolve_config_root(canonical_data_root)
                            / AUTH_BACKOFF_FILENAME
                        )
                    ),
                )
                backoff.check(scope)
                try:
                    connection = open_db(
                        canonical_data_root,
                        passphrase=_decode_secret(secret),
                        require_existing_schema=True,
                    )
                    try:
                        authenticated_database_identity = (
                            database_instance_id(connection)
                            if hasattr(connection, "execute")
                            else project.identity
                        )
                    finally:
                        connection.close()
                except AppError as exc:
                    if exc.code in {"unlock_failed", "passphrase_required"}:
                        backoff.record_failure()
                    raise
                else:
                    backoff.record_success()
                current_project = canonical_project(canonical_data_root)
                if current_project.identity != project.identity:
                    raise AppError(
                        "the project changed during fresh authentication",
                        code="operator_project_replaced",
                        retryable=False,
                    )
                active_lease = self._leases.get(project.identity)
                if (
                    active_lease is not None
                    and active_lease.database_identity
                    != authenticated_database_identity
                ):
                    raise AppError(
                        "fresh authentication opened a different project database",
                        code="operator_project_replaced",
                        retryable=False,
                    )
                return continuation() if continuation is not None else None
            finally:
                if temporary_owner is not None:
                    temporary_owner.release()

    def set_mode_authenticated(
        self,
        data_root: str,
        secret: bytearray,
        mode: str,
    ) -> dict[str, object]:
        project = canonical_project(data_root)
        canonical_data_root = str(project.database.parent)

        def apply_mode() -> str:
            selected = set_unlock_mode(canonical_data_root, mode)
            if selected != "brokered":
                project_identity = self._lease_identity_for_path_locked(project)
                self._revoke_lease_locked(
                    project_identity,
                    reason="operator mode ended the brokered lease",
                )
            return selected

        selected = self.authenticate_database(
            canonical_data_root,
            secret,
            scope="operator_mode",
            require_lease=False,
            continuation=apply_mode,
        )
        assert isinstance(selected, str)
        return {"mode": selected, "project": project.public_id}

    def configure_touch_id_authenticated(
        self,
        data_root: str,
        secret: bytearray,
        *,
        configured: bool,
    ) -> dict[str, object]:
        project = canonical_project(data_root)
        canonical_data_root = str(project.database.parent)

        def configure() -> None:
            from .native_auth import touch_id_delete, touch_id_store

            if configured:
                touch_id_store(canonical_data_root, secret)
            else:
                touch_id_delete(canonical_data_root)

        self.authenticate_database(
            canonical_data_root,
            secret,
            scope="operator_native_enrollment",
            require_lease=False,
            continuation=configure,
        )
        return {"configured": configured, "auth": "touch_id"}

    def lock(self, data_root: str) -> dict[str, object]:
        project = canonical_project(data_root)
        with self._lock:
            project_identity = self._lease_identity_for_path_locked(project)
            existed = project_identity in self._leases
            running = 0
            if existed:
                lease = self._leases[project_identity]
                running = lease.running_operations
                self._revoke_lease_locked(
                    project_identity,
                    reason="operator lease was locked",
                )
        _LOGGER.info(
            "operator lease locked",
            extra={
                "kb_fields": {
                    "project": project.public_id,
                    "running_operations_finishing": running,
                }
            },
        )
        return {
            "project": project.public_id,
            "locked": True,
            "lease_existed": existed,
            "running_operations_finishing": running,
            "generation": self.generation,
        }

    def status(self, data_root: str | None = None) -> dict[str, object]:
        with self._lock:
            self._drop_expired_locked()
            if data_root is None:
                leases = [
                    self._lease_status(lease)
                    for lease in self._leases.values()
                    if not lease.revoked
                ]
                return {
                    "broker": "running",
                    "generation": self.generation,
                    "leases": leases,
                }
            project = canonical_project(data_root)
            project_identity = self._lease_identity_for_path_locked(project)
            lease = self._leases.get(project_identity)
            if lease is None or lease.revoked:
                return {
                    "broker": "running",
                    "generation": self.generation,
                    "project": project.public_id,
                    "lease": "locked",
                }
            if project_identity == project.identity:
                lease.owner.add_alias(project)
                lease.project = project
                self._lease_aliases[str(project.database)] = project.identity
            payload = {
                "broker": "running",
                "generation": self.generation,
                **self._lease_status(lease),
            }
            if project_identity != project.identity:
                payload["project_file_changed"] = True
            return payload

    def submit(
        self,
        data_root: str,
        argv: list[str],
        *,
        operation_id: str | None = None,
        secret_arguments: dict[str, bytearray] | None = None,
        admin_verified: bool = False,
    ) -> dict[str, object]:
        parsed, command_path, required = _parse_argv(argv)
        if command_path.startswith("operator.") or command_path in {"daemon", "chat"}:
            raise AppError(
                "this command cannot run inside an operator worker",
                code="operator_command_not_brokerable",
                details={"command": command_path},
                retryable=False,
            )
        project = canonical_project(data_root)
        try:
            with self._lock:
                admitted_identity = self._lease_identity_for_path_locked(project)
                if admitted_identity != project.identity:
                    raise AppError(
                        "the database file changed while its operator lease is active",
                        code="operator_project_replaced",
                        hint=(
                            "Lock the prior lease before operating on the "
                            "replacement database."
                        ),
                        retryable=False,
                    )
                self._require_lease_locked(project)
        except Exception:
            for value in (secret_arguments or {}).values():
                _wipe(value)
            raise
        _require_explicit_scope(
            parsed,
            command_path,
            secret_arguments,
        )
        canonical_data_root = str(project.database.parent)
        installs_backup = command_path == "backup.import" and bool(
            getattr(parsed, "install", False)
        )
        if installs_backup:
            for value in (secret_arguments or {}).values():
                _wipe(value)
            raise AppError(
                "backup installation cannot run inside an operator worker",
                code="operator_command_not_brokerable",
                hint=(
                    "Lock the broker lease, select manual mode, and run the "
                    "restore explicitly for its destination."
                ),
                details={
                    "lease_project": project.public_id,
                    "command": command_path,
                },
                retryable=False,
            )
        explicit_data_root = getattr(parsed, "data_root", None)
        explicit_project = getattr(parsed, "project", None)
        if explicit_data_root is not None or explicit_project is not None:
            requested_paths = resolve_runtime_paths(
                explicit_data_root,
                getattr(parsed, "env_file", None),
                explicit_project,
            )
            requested_project = canonical_project(requested_paths.data_root)
            if requested_project.identity != project.identity:
                for value in (secret_arguments or {}).values():
                    _wipe(value)
                raise AppError(
                    "the command targets a different project than the operator lease",
                    code="operator_project_mismatch",
                    details={
                        "lease_project": project.public_id,
                        "command_project": requested_project.public_id,
                    },
                    retryable=False,
                )
        with self._lock:
            admitted_identity = self._lease_identity_for_path_locked(project)
            if admitted_identity != project.identity:
                for value in (secret_arguments or {}).values():
                    _wipe(value)
                raise AppError(
                    "the database file changed while its operator lease is active",
                    code="operator_project_replaced",
                    hint="Lock the prior lease before operating on the replacement database.",
                    retryable=False,
                )
            lease = self._require_lease_locked(project)
            lease.owner.add_alias(project)
            lease.data_root = canonical_data_root
            lease.project = project
            self._lease_aliases[str(project.database)] = project.identity
            pinned_argv = _pin_project_arguments(
                argv,
                canonical_data_root,
            )
            if operation_id is not None:
                if (
                    not operation_id.startswith(f"{self.generation}.")
                    or len(operation_id) > 160
                ):
                    raise AppError(
                        "invalid proposed operation id",
                        code="operator_protocol_error",
                        retryable=False,
                    )
                existing = self._operations.get(operation_id)
                if existing is not None:
                    for value in (secret_arguments or {}).values():
                        _wipe(value)
                    if (
                        existing.project_id != project.public_id
                        or existing.argv != pinned_argv
                    ):
                        raise AppError(
                            "the operation id is already bound to another request",
                            code="operator_operation_id_conflict",
                            retryable=False,
                        )
                    return existing.public_status()
                tombstone = self._operation_tombstones.get(operation_id)
                if tombstone is not None:
                    for value in (secret_arguments or {}).values():
                        _wipe(value)
                    bound_project, bound_argv = tombstone
                    if bound_project != project.public_id or bound_argv != tuple(pinned_argv):
                        raise AppError(
                            "the operation id is already bound to another request",
                            code="operator_operation_id_conflict",
                            retryable=False,
                        )
                    self._operation_tombstones.move_to_end(operation_id)
                    return {
                        "operation_id": operation_id,
                        "state": "result_unknown",
                        "reason": "result_not_retained",
                        "generation": self.generation,
                    }
            if required is Capability.ADMIN:
                if not admin_verified:
                    raise AppError(
                        "admin commands require fresh authentication",
                        code="operator_admin_auth_required",
                        details={"command": command_path},
                        retryable=False,
                    )
            elif not capability_allows(lease.capability, required):
                raise AppError(
                    "the operator lease does not grant this command",
                    code="operator_capability_denied",
                    details={
                        "command": command_path,
                        "required": required.value,
                        "granted": lease.capability.value,
                    },
                    retryable=False,
                )
            self._sequence += 1
            operation_id = operation_id or (
                f"{self.generation}.{project.public_id}.{self._sequence}."
                f"{secrets.token_hex(4)}"
            )
            operation = Operation(
                id=operation_id,
                generation=self.generation,
                project_id=project.public_id,
                project_identity=project.identity,
                database_identity=lease.database_identity,
                data_root=canonical_data_root,
                argv=pinned_argv,
                command_path=command_path,
                capability=required,
                secret_arguments=secret_arguments or {},
                admin_authorized_until_monotonic=(
                    time.monotonic() + ADMIN_AUTH_TTL_SECONDS
                    if required is Capability.ADMIN and admin_verified
                    else None
                ),
            )
            worker = self._workers.setdefault(
                project.identity,
                ProjectWorker(
                    self,
                    project.identity,
                    project.public_id,
                    self._runner,
                ),
            )
            try:
                worker.submit(operation)
            except Exception:
                for value in operation.secret_arguments.values():
                    _wipe(value)
                operation.secret_arguments.clear()
                raise
            self._operations[operation_id] = operation
            self._prune_operations_locked()
            _LOGGER.info(
                "operator operation admitted",
                extra={
                    "kb_fields": {
                        "project": project.public_id,
                        "command": command_path,
                        "capability": required.value,
                        "state": "queued",
                    }
                },
            )
            return operation.public_status()

    def operation_status(
        self,
        operation_id: str,
        *,
        include_output: bool = True,
    ) -> dict[str, object]:
        if not operation_id.startswith(f"{self.generation}."):
            return {
                "operation_id": operation_id,
                "state": "result_unknown",
                "reason": "broker_generation_changed",
                "generation": self.generation,
            }
        with self._lock:
            operation = self._operations.get(operation_id)
            if operation is None:
                return {
                    "operation_id": operation_id,
                    "state": "result_unknown",
                    "reason": "result_not_retained",
                    "generation": self.generation,
                }
            return operation.public_status(include_output=include_output)

    def cancel(self, operation_id: str) -> dict[str, object]:
        with self._lock:
            operation = self._operations.get(operation_id)
            if operation is None:
                return self.operation_status(operation_id)
            if operation.state in {
                "completed",
                "failed",
                "cancelled",
                "result_unknown",
            }:
                return operation.public_status(include_output=True)
            if operation.state == "queued":
                operation.cancellation_requested = True
                self._finish_operation_locked(operation, "cancelled", None)
                worker = self._worker_for_public_project_locked(operation.project_id)
                if worker is not None:
                    worker.remove(operation)
                _LOGGER.info(
                    "operator queued operation cancelled",
                    extra={
                        "kb_fields": {
                            "project": operation.project_id,
                            "command": operation.command_path,
                            "state": "cancelled",
                        }
                    },
                )
            else:
                payload = operation.public_status(include_output=True)
                payload["cancellation"] = "not_cancellable"
                payload["reason"] = "running_operation_has_no_cooperative_cancel_contract"
                return payload
            return operation.public_status(include_output=True)

    def close(self) -> None:
        self._closed.set()
        with self._lock:
            for project_id in list(self._leases):
                self._revoke_lease_locked(
                    project_id,
                    reason="operator broker stopped",
                )
            for worker in self._workers.values():
                worker.stop()

    def _require_lease_locked(self, project: CanonicalProject) -> ProjectLease:
        lease = self._leases.get(project.identity)
        if lease is None or lease.revoked or lease.expired():
            if lease is not None:
                lease.revoked = True
                if lease.running_operations == 0:
                    self._drop_lease_locked(project.identity)
            raise AppError(
                "this project has no active operator lease",
                code="interaction_required",
                hint="Run `kassiber operator unlock` in a terminal.",
                details={"project": project.public_id},
                retryable=True,
            )
        return lease

    def _finish_operation_locked(
        self,
        operation: Operation,
        state: str,
        result: OperationResult | None,
    ) -> None:
        operation.state = state
        operation.result = result
        operation.finished_at = now_iso()
        operation.process = None
        for secret in operation.secret_arguments.values():
            _wipe(secret)
        operation.secret_arguments.clear()
        if operation.id in self._operations:
            self._operations.move_to_end(operation.id)
        self._prune_operations_locked()
        with operation.changed:
            operation.changed.notify_all()

    def _drop_lease_locked(self, project_id: str) -> None:
        lease = self._leases.pop(project_id, None)
        if lease is None:
            return
        worker = self._workers.pop(project_id, None)
        if worker is not None:
            worker.stop()
        _wipe(lease.passphrase)
        lease.owner.release()
        for alias, identity in list(self._lease_aliases.items()):
            if identity == project_id:
                self._lease_aliases.pop(alias, None)

    def _revoke_lease_locked(self, project_id: str, *, reason: str) -> None:
        lease = self._leases.get(project_id)
        if lease is None:
            return
        lease.revoked = True
        worker = self._workers.get(project_id)
        if worker is not None:
            worker.drain()
        for operation in list(self._operations.values()):
            if operation.project_id != lease.project.public_id:
                continue
            if operation.state != "queued":
                continue
            operation.cancellation_requested = True
            self._finish_operation_locked(
                operation,
                "cancelled",
                OperationResult(1, "", f"{reason}\n"),
            )
        if lease.running_operations == 0:
            self._drop_lease_locked(project_id)

    def _lease_identity_for_path_locked(self, project: CanonicalProject) -> str:
        return self._lease_aliases.get(str(project.database), project.identity)

    def _worker_for_public_project_locked(
        self,
        public_id: str,
    ) -> ProjectWorker | None:
        return next(
            (worker for worker in self._workers.values() if worker.project_id == public_id),
            None,
        )

    def _refresh_scope_locked(self, lease: ProjectLease) -> None:
        try:
            connection = open_db(
                lease.data_root,
                passphrase=_decode_secret(lease.passphrase),
                require_existing_schema=True,
                expected_database_identity=lease.database_identity,
            )
            try:
                context = current_context_snapshot(connection)
            finally:
                connection.close()
        except Exception as exc:
            _LOGGER.warning(
                "operator scope refresh failed",
                extra={
                    "kb_fields": {
                        "project": lease.project.public_id,
                        "error": sanitize_traceback_text(str(exc)),
                    }
                },
            )
            self._revoke_lease_locked(
                lease.project.identity,
                reason="operator scope could not be refreshed",
            )
            return
        lease.workspace = (
            str(context.get("workspace_id")) if context.get("workspace_id") else None
        )
        lease.profile = (
            str(context.get("profile_id")) if context.get("profile_id") else None
        )

    def _drop_expired_locked(self) -> None:
        for project_id, lease in list(self._leases.items()):
            if lease.expired():
                lease.revoked = True
                _LOGGER.info(
                    "operator lease expired",
                    extra={
                        "kb_fields": {
                            "project": lease.project.public_id,
                            "running_operations_finishing": lease.running_operations,
                        }
                    },
                )
                self._revoke_lease_locked(
                    project_id,
                    reason="operator lease expired",
                )

    def _lease_status(self, lease: ProjectLease) -> dict[str, object]:
        remaining = (
            max(0, int(lease.expires_at_monotonic - time.monotonic()))
            if lease.expires_at_monotonic is not None
            else None
        )
        worker = self._workers.get(lease.project.identity)
        queued = worker.queued if worker is not None else 0
        worker_state = (
            "running"
            if lease.running_operations
            else "queued"
            if queued
            else "idle"
        )
        return {
            "project": lease.project.public_id,
            "lease": "unlocked",
            "capability": lease.capability.value,
            "granted_capabilities": [
                capability.value
                for capability in (
                    Capability.READ,
                    Capability.OPERATOR,
                    Capability.ACCOUNTING_DECISIONS,
                )
                if capability_allows(lease.capability, capability)
            ],
            "authentication_method": lease.authentication_method,
            "unlocked_at": lease.unlocked_at,
            "duration_seconds": lease.duration_seconds,
            "expires_at": lease.expires_at,
            "remaining_seconds": remaining,
            "until_lock": lease.expires_at_monotonic is None,
            "queued_operations": queued,
            "running_operations": lease.running_operations,
            "worker_state": worker_state,
            "default_scope": {
                "workspace": lease.workspace,
                "profile": lease.profile,
            },
        }

    def _prune_operations_locked(self) -> None:
        terminal_ids = [
            operation_id
            for operation_id, operation in self._operations.items()
            if operation.state
            in {
                "completed",
                "failed",
                "cancelled",
                "result_unknown",
            }
        ]
        for removable in terminal_ids[:-MAX_RETAINED_RESULTS]:
            operation = self._operations.pop(removable, None)
            if operation is not None:
                self._operation_tombstones[removable] = (
                    operation.project_id,
                    tuple(operation.argv),
                )
                self._operation_tombstones.move_to_end(removable)
        while len(self._operation_tombstones) > MAX_RETAINED_OPERATION_TOMBSTONES:
            self._operation_tombstones.popitem(last=False)

    def _expire_leases(self) -> None:
        while not self._closed.wait(1.0):
            with self._lock:
                self._drop_expired_locked()


def _classify_argv(argv: list[str]) -> tuple[str, Capability]:
    _args, path, capability = _parse_argv(argv)
    return path, capability


def _require_explicit_scope(
    parsed: object,
    command_path: str,
    secret_arguments: dict[str, bytearray] | None,
) -> None:
    has_workspace = hasattr(parsed, "workspace")
    has_profile = hasattr(parsed, "profile")
    workspace = getattr(parsed, "workspace", None) if has_workspace else None
    profile = getattr(parsed, "profile", None) if has_profile else None
    if command_path == "context.set":
        missing = [] if workspace or profile else ["workspace_or_profile"]
    else:
        missing = [
            name
            for name, declared, value in (
                ("workspace", has_workspace, workspace),
                ("profile", has_profile, profile),
            )
            if declared and not value
        ]
    if missing:
        for value in (secret_arguments or {}).values():
            _wipe(value)
        raise AppError(
            "brokered commands require explicit book scope",
            code="operator_scope_required",
            hint="Pass the command's --workspace and --profile flags explicitly.",
            details={"command": command_path, "missing": missing},
            retryable=False,
        )


def _parse_argv(
    argv: list[str],
) -> tuple[object, str, Capability]:
    from ..cli.main import build_parser
    from ..cli.command_registry import command_path

    parser = build_parser()
    try:
        args = parser.parse_args(_classification_argv(argv))
    except SystemExit as exc:
        raise AppError(
            "the broker received invalid command arguments",
            code="operator_invalid_command",
            retryable=False,
        ) from exc
    path = command_path(args)
    return args, path, cli_capability(path)


def _classification_argv(argv: list[str]) -> list[str]:
    normalized = list(argv)
    for index in range(1, len(normalized)):
        if (
            normalized[index].startswith("broker-secret-")
            and normalized[index - 1].startswith("--")
            and normalized[index - 1].endswith("-fd")
        ):
            normalized[index] = "0"
    return normalized


def _pin_project_arguments(
    argv: list[str],
    data_root: str,
) -> list[str]:
    pinned = ["--data-root", data_root]
    index = 0
    while index < len(argv):
        token = argv[index]
        if token in {"--project", "--data-root"}:
            index += 2
            continue
        if token.startswith(("--project=", "--data-root=")):
            index += 1
            continue
        pinned.append(token)
        index += 1
    return pinned


def _decode_secret(secret: bytearray) -> str:
    try:
        return bytes(secret).decode("utf-8")
    except UnicodeDecodeError as exc:
        raise AppError(
            "database passphrase must be UTF-8",
            code="invalid_passphrase_encoding",
            retryable=False,
        ) from exc


def _wipe(value: bytearray) -> None:
    for index in range(len(value)):
        value[index] = 0
