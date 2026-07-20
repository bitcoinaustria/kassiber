"""In-memory leases, per-project queues, and recoverable operation state."""

from __future__ import annotations

import hashlib
import json
import logging
import secrets
import subprocess
import threading
import time
from collections import OrderedDict, deque
from contextlib import contextmanager
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Callable, Iterator

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
from .protocol import MAX_JSON_FRAME


MAX_QUEUED_OPERATIONS = 64
MAX_RETAINED_RESULTS = 256
MAX_RETAINED_RESULT_BYTES = 16 * 1024 * 1024
MAX_RETAINED_OPERATION_TOMBSTONES = 1024
MAX_CACHED_AUTH_BACKOFFS = 256
ADMIN_AUTH_TTL_SECONDS = 60.0
_OPERATION_STATUS_FRAME_HEADROOM = 64 * 1024
_LOGGER = logging.getLogger("kassiber.operator")
OperationRunner = Callable[["Operation", bytearray], "OperationResult"]
_LEASE_ENDING_COMMAND_REASONS = {
    "secrets.change-passphrase": (
        "database passphrase rotation ended the prior lease"
    ),
    "secrets.remember-unlock": (
        "remembered unlock enrollment ended the prior brokered lease"
    ),
    "secrets.forget-unlock": (
        "remembered unlock removal ended the prior brokered lease"
    ),
}


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
    secret_arguments: dict[str, bytearray] = field(repr=False)
    admin_authorized_until_monotonic: float | None = None
    state: str = "queued"
    submitted_at: str = field(default_factory=now_iso)
    started_at: str | None = None
    finished_at: str | None = None
    result: OperationResult | None = None
    output_error: dict[str, object] | None = None
    retained_result_bytes: int = 0
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
            if self.output_error is not None:
                payload["output_available"] = False
                payload["output_error"] = dict(self.output_error)
            elif include_output:
                payload["stdout"] = self.result.stdout
                payload["stderr"] = self.result.stderr
        return payload


@dataclass
class ProjectLease:
    data_root: str
    project: CanonicalProject
    database_identity: str
    passphrase: bytearray = field(repr=False)
    capability: Capability
    owner: ProjectOwnerLease
    unlocked_at: str
    expires_at_monotonic: float | None
    duration_seconds: int | None
    authentication_method: str
    expires_at: str | None
    epoch: str = field(default_factory=lambda: secrets.token_urlsafe(24))
    workspace: str | None = None
    profile: str | None = None
    revoked: bool = False
    running_operations: int = 0

    def expired(self) -> bool:
        return (
            self.expires_at_monotonic is not None
            and time.monotonic() >= self.expires_at_monotonic
        )


@dataclass
class AdminAuthorization:
    """Single-use fresh authorization bound to one live project lease."""

    project_identity: str
    database_identity: str
    lease_epoch: str
    expires_at_monotonic: float
    consumed: bool = field(default=False, repr=False)


@dataclass
class _ProjectTransitionGate:
    """Serialize state transitions for one canonical project identity."""

    lock: threading.RLock = field(default_factory=threading.RLock)
    users: int = 0


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
            try:
                self._run_one(operation)
            except Exception as exc:
                self._recover_unhandled_exception(operation, exc)

    def _recover_unhandled_exception(
        self,
        operation: Operation,
        exc: Exception,
    ) -> None:
        result = OperationResult(
            1,
            "",
            sanitize_traceback_text(f"operator worker failed: {exc}\n"),
        )
        with self._service._project_transition(
            self.project_identity,
            allow_closed=True,
        ):
            with self._service._lock:
                lease = self._service._leases.get(self.project_identity)
                if operation.state == "running" and lease is not None:
                    lease.running_operations = max(0, lease.running_operations - 1)
                if operation.state not in {
                    "completed",
                    "failed",
                    "cancelled",
                    "result_unknown",
                }:
                    self._service._finish_operation_locked(
                        operation,
                        "result_unknown",
                        result,
                    )
                _LOGGER.error(
                    "operator worker recovered from an internal failure",
                    extra={
                        "kb_fields": {
                            "project": self.project_id,
                            "command": operation.command_path,
                            "state": operation.state,
                        }
                    },
                )
                if (
                    lease is not None
                    and (lease.revoked or lease.expired())
                    and lease.running_operations == 0
                ):
                    self._service._drop_lease_locked(self.project_identity)

    def _run_one(self, operation: Operation) -> None:
        with self._service._project_transition(
            self.project_identity,
            allow_closed=True,
        ):
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
                    and not capability_allows(
                        lease.capability,
                        operation.capability,
                    )
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
                if operation.capability is Capability.ADMIN and (
                    operation.admin_authorized_until_monotonic is None
                    or time.monotonic()
                    > operation.admin_authorized_until_monotonic
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
                current_project = canonical_project(operation.data_root)
            except Exception:
                current_project = None
            if (
                current_project is None
                or current_project.identity != operation.project_identity
            ):
                with self._service._lock:
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
            try:
                inherited_owner = lease.owner.duplicate_for_child()
            except Exception as exc:
                with self._service._lock:
                    self._service._finish_operation_locked(
                        operation,
                        "failed",
                        OperationResult(
                            1,
                            "",
                            sanitize_traceback_text(
                                f"operator ownership inheritance failed: {exc}\n"
                            ),
                        ),
                    )
                return
            with self._service._lock:
                current_lease = self._service._leases.get(self.project_identity)
                admin_authorization_expired = (
                    operation.capability is Capability.ADMIN
                    and (
                        operation.admin_authorized_until_monotonic is None
                        or time.monotonic()
                        > operation.admin_authorized_until_monotonic
                    )
                )
                dispatch_rejected = (
                    current_lease is not lease
                    or lease.revoked
                    or lease.expired()
                    or operation.cancellation_requested
                    or admin_authorization_expired
                )
                if dispatch_rejected:
                    self._service._finish_operation_locked(
                        operation,
                        "cancelled",
                        OperationResult(
                            1,
                            "",
                            (
                                "fresh admin authentication expired before dispatch\n"
                                if admin_authorization_expired
                                else "operator lease ended before the operation started\n"
                            ),
                        ),
                    )
                else:
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
            if dispatch_rejected:
                inherited_owner.close()
                return
        runner_crashed = False
        cleanup_error: Exception | None = None
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
            try:
                inherited_owner.close()
            except Exception as exc:
                cleanup_error = exc
        result = OperationResult(
            result.exit_code,
            result.stdout,
            redact_secret_text(result.stderr),
        )
        with self._service._project_transition(
            self.project_identity,
            allow_closed=True,
        ):
            with self._service._lock:
                current_lease = self._service._leases.get(self.project_identity)
                if current_lease is not None:
                    current_lease.running_operations = max(
                        0, current_lease.running_operations - 1
                    )
                    if cleanup_error is not None:
                        current_lease.revoked = True
                        _LOGGER.error(
                            "operator ownership cleanup failed; lease revoked",
                            extra={
                                "kb_fields": {
                                    "project": self.project_id,
                                    "command": operation.command_path,
                                }
                            },
                        )
                if runner_crashed or result.exit_code < 0:
                    state = "result_unknown"
                elif (
                    result.exit_code != 0
                    and operation.capability is not Capability.READ
                ):
                    state = "result_unknown"
                else:
                    state = "completed" if result.exit_code == 0 else "failed"
                self._service._finish_operation_locked(operation, state, result)
                lease_end_reason = _LEASE_ENDING_COMMAND_REASONS.get(
                    operation.command_path
                )
                if lease_end_reason is not None:
                    self._service._revoke_lease_locked(
                        self.project_identity,
                        reason=lease_end_reason,
                    )
                refresh_scope = (
                    operation.command_path == "context.set"
                    and state == "completed"
                    and current_lease is not None
                    and not current_lease.revoked
                )
            if refresh_scope:
                assert current_lease is not None
                self._service._refresh_scope(
                    self.project_identity,
                    current_lease,
                )
            with self._service._lock:
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
                current_lease = self._service._leases.get(self.project_identity)
                if (
                    current_lease is not None
                    and (current_lease.revoked or current_lease.expired())
                    and current_lease.running_operations == 0
                ):
                    self._service._drop_lease_locked(self.project_identity)
            self._service._release_pending_owners(self.project_identity)


class OperatorService:
    def __init__(self, generation: str, runner: OperationRunner) -> None:
        self.generation = generation
        self._runner = runner
        self._lock = threading.RLock()
        self._leases: dict[str, ProjectLease] = {}
        self._workers: dict[str, ProjectWorker] = {}
        self._operations: OrderedDict[str, Operation] = OrderedDict()
        self._operation_tombstones: OrderedDict[
            str, tuple[str, str]
        ] = OrderedDict()
        self._lease_aliases: dict[str, str] = {}
        self._auth_backoffs: OrderedDict[str, AuthAttemptBackoff] = OrderedDict()
        self._project_gates: dict[str, _ProjectTransitionGate] = {}
        self._pending_owner_releases: deque[
            tuple[str, ProjectOwnerLease]
        ] = deque()
        self._active_project_transitions = 0
        self._sequence = 0
        self._closed = threading.Event()
        self._close_complete = threading.Event()
        self._close_in_progress = False
        self._close_prepared = False
        self._close_terminal_error: BaseException | None = None
        self._transition_condition = threading.Condition(self._lock)
        self._janitor = threading.Thread(
            target=self._expire_leases,
            name="operator-lease-expiry",
            daemon=True,
        )
        self._janitor.start()

    @contextmanager
    def _project_transition(
        self,
        project_identity: str,
        *,
        allow_closed: bool = False,
    ) -> Iterator[None]:
        """Hold a project-local gate without holding the service state lock."""

        with self._lock:
            if self._closed.is_set() and not allow_closed:
                raise _broker_stopped_error()
            gate = self._project_gates.setdefault(
                project_identity,
                _ProjectTransitionGate(),
            )
            gate.users += 1
            if not allow_closed:
                self._active_project_transitions += 1
        try:
            with gate.lock:
                try:
                    yield
                finally:
                    self._release_pending_owners(project_identity)
        finally:
            with self._lock:
                gate.users -= 1
                if not allow_closed:
                    self._active_project_transitions -= 1
                    self._transition_condition.notify_all()
                if (
                    gate.users == 0
                    and project_identity not in self._leases
                    and not any(
                        identity == project_identity
                        for identity, _owner in self._pending_owner_releases
                    )
                ):
                    self._project_gates.pop(project_identity, None)

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
        with self._project_transition(project.identity):
            self._release_pending_owners(project.identity)
            with self._lock:
                replaced_identity = self._lease_aliases.get(alias)
                if (
                    replaced_identity is not None
                    and replaced_identity != project.identity
                ):
                    raise AppError(
                        "the database file changed while its prior operator lease is active",
                        code="operator_project_replaced",
                        hint=(
                            "Lock the prior lease before unlocking the replacement "
                            "database."
                        ),
                        retryable=False,
                    )
                previous = self._leases.get(project.identity)
                if previous is not None and (previous.revoked or previous.expired()):
                    self._revoke_lease_locked(
                        project.identity,
                        reason=(
                            "operator lease expired"
                            if previous.expired()
                            else "operator lease was revoked"
                        ),
                    )
                    if previous.running_operations == 0:
                        previous = None
                if previous is not None and previous.running_operations > 0:
                    raise AppError(
                        "the project still has a running operator operation",
                        code="operator_project_busy",
                        hint=(
                            "Wait for the running operation to finish, then refresh "
                            "the lease."
                        ),
                        details={"project": project.public_id},
                        retryable=True,
                    )
                backoff = self._auth_backoff_locked(
                    project.identity,
                    canonical_data_root,
                )
            self._release_pending_owners(project.identity)
            acquired_here = previous is None
            owner = (
                previous.owner
                if previous is not None
                else acquire_project_ownership(
                    project,
                    owner_kind="broker",
                    generation=self.generation,
                )
            )
            try:
                if previous is not None:
                    owner.add_alias(project)
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
            current_project = canonical_project(canonical_data_root)
            if current_project.identity != project.identity:
                if acquired_here:
                    owner.release()
                raise AppError(
                    "the project changed during operator unlock",
                    code="operator_project_replaced",
                    retryable=False,
                )
            if (
                previous is not None
                and previous.database_identity != opened_database_identity
            ):
                raise AppError(
                    "operator unlock opened a different project database",
                    code="operator_project_replaced",
                    retryable=False,
                )
            stored = bytearray(passphrase)
            expires = (
                time.monotonic() + duration_seconds
                if duration_seconds is not None
                else None
            )
            with self._lock:
                current_previous = self._leases.get(project.identity)
                lease_changed = current_previous is not previous
                service_stopped = self._closed.is_set()
                if not lease_changed and not service_stopped:
                    if previous is not None:
                        _wipe(previous.passphrase)
                    self._leases[project.identity] = ProjectLease(
                        data_root=canonical_data_root,
                        project=current_project,
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
                    if project.identity not in self._workers:
                        self._workers[project.identity] = ProjectWorker(
                            self,
                            project.identity,
                            project.public_id,
                            self._runner,
                        )
            if lease_changed or service_stopped:
                _wipe(stored)
                if acquired_here:
                    owner.release()
                if service_stopped:
                    raise _broker_stopped_error()
                raise AppError(
                    "the operator lease changed during unlock",
                    code="operator_project_busy",
                    retryable=True,
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

    def verify_admin(
        self,
        data_root: str,
        secret: bytearray,
    ) -> AdminAuthorization:
        project = canonical_project(data_root)

        def issue_authorization() -> AdminAuthorization:
            with self._lock:
                lease = self._require_lease_locked(project)
                return AdminAuthorization(
                    project_identity=project.identity,
                    database_identity=lease.database_identity,
                    lease_epoch=lease.epoch,
                    expires_at_monotonic=(
                        time.monotonic() + ADMIN_AUTH_TTL_SECONDS
                    ),
                )

        authorization = self.authenticate_database(
            data_root,
            secret,
            scope="operator_admin",
            continuation=issue_authorization,
        )
        assert isinstance(authorization, AdminAuthorization)
        return authorization

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
        with self._project_transition(project.identity):
            self._release_pending_owners(project.identity)
            temporary_owner: ProjectOwnerLease | None = None
            with self._lock:
                if require_lease:
                    existing = self._require_lease_locked(project)
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
                backoff = self._auth_backoff_locked(
                    project.identity,
                    canonical_data_root,
                )
            self._release_pending_owners(project.identity)
            if existing is None:
                temporary_owner = acquire_project_ownership(
                    project,
                    owner_kind="broker",
                    generation=self.generation,
                )
            try:
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
                with self._lock:
                    if require_lease:
                        active_lease = self._require_lease_locked(current_project)
                        if active_lease is not existing:
                            raise AppError(
                                "the operator lease changed during authentication",
                                code="operator_project_busy",
                                retryable=True,
                            )
                    else:
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
                    if self._closed.is_set():
                        raise _broker_stopped_error()
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
                with self._lock:
                    project_identity = self._lease_identity_for_path_locked(project)
                    self._revoke_lease_locked(
                        project_identity,
                        reason="operator mode ended the brokered lease",
                    )
                self._release_pending_owners(project.identity)
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
            transition_identity = self._lease_identity_for_path_locked(project)
        with self._project_transition(transition_identity):
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
            self._release_pending_owners(project_identity)
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
        project = canonical_project(data_root) if data_root is not None else None
        if project is not None:
            with self._lock:
                transition_identity = self._lease_identity_for_path_locked(project)
            with self._project_transition(transition_identity):
                return self._project_status(project)
        with self._lock:
            leases = [
                self._lease_status(lease)
                for lease in self._leases.values()
                if not lease.revoked and not lease.expired()
            ]
            return {
                "broker": "running",
                "generation": self.generation,
                "leases": leases,
            }

    def _project_status(self, project: CanonicalProject) -> dict[str, object]:
        self._release_pending_owners(project.identity)
        with self._lock:
            project_identity = self._lease_identity_for_path_locked(project)
            lease = self._leases.get(project_identity)
            if lease is not None and lease.expired():
                self._revoke_lease_locked(
                    project_identity,
                    reason="operator lease expired",
                )
                lease = None
        self._release_pending_owners(project_identity)
        with self._lock:
            lease = self._leases.get(project_identity) if lease is not None else None
            if lease is None or lease.revoked:
                return {
                    "broker": "running",
                    "generation": self.generation,
                    "project": project.public_id,
                    "lease": "locked",
                }
            should_add_alias = project_identity == project.identity
        if should_add_alias:
            lease.owner.add_alias(project)
            with self._lock:
                if self._leases.get(project_identity) is not lease or lease.revoked:
                    return {
                        "broker": "running",
                        "generation": self.generation,
                        "project": project.public_id,
                        "lease": "locked",
                    }
                lease.project = project
                self._lease_aliases[str(project.database)] = project.identity
        with self._lock:
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
        admin_authorization: AdminAuthorization | None = None,
    ) -> dict[str, object]:
        owned_secrets = secret_arguments if secret_arguments is not None else {}
        ownership_transferred = False

        def transfer_secret_ownership() -> None:
            nonlocal ownership_transferred
            ownership_transferred = True

        try:
            return self._submit(
                data_root,
                argv,
                operation_id=operation_id,
                secret_arguments=owned_secrets,
                admin_authorization=admin_authorization,
                transfer_secret_ownership=transfer_secret_ownership,
            )
        finally:
            if not ownership_transferred:
                _wipe_secret_arguments(owned_secrets)

    def _submit(
        self,
        data_root: str,
        argv: list[str],
        *,
        operation_id: str | None,
        secret_arguments: dict[str, bytearray],
        admin_authorization: AdminAuthorization | None,
        transfer_secret_ownership: Callable[[], None],
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
        with self._project_transition(project.identity):
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
        _require_explicit_scope(
            parsed,
            command_path,
        )
        canonical_data_root = str(project.database.parent)
        installs_backup = command_path == "backup.import" and bool(
            getattr(parsed, "install", False)
        )
        if installs_backup:
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
                raise AppError(
                    "the command targets a different project than the operator lease",
                    code="operator_project_mismatch",
                    details={
                        "lease_project": project.public_id,
                        "command_project": requested_project.public_id,
                    },
                    retryable=False,
                )
        with self._project_transition(project.identity):
            self._release_pending_owners(project.identity)
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
                alias_lease = self._require_lease_locked(project)
            alias_lease.owner.add_alias(project)
        with self._lock:
            admitted_identity = self._lease_identity_for_path_locked(project)
            if admitted_identity != project.identity:
                raise AppError(
                    "the database file changed while its operator lease is active",
                    code="operator_project_replaced",
                    hint="Lock the prior lease before operating on the replacement database.",
                    retryable=False,
                )
            lease = self._require_lease_locked(project)
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
                    bound_project, bound_fingerprint = tombstone
                    if (
                        bound_project != project.public_id
                        or bound_fingerprint
                        != _operation_request_fingerprint(pinned_argv)
                    ):
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
                admin_authorized_until = (
                    self._consume_admin_authorization_locked(
                        admin_authorization,
                        project=project,
                        lease=lease,
                        command_path=command_path,
                    )
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
                secret_arguments=secret_arguments,
                admin_authorized_until_monotonic=(
                    admin_authorized_until
                    if required is Capability.ADMIN
                    else None
                ),
            )
            worker = self._workers.get(project.identity)
            if worker is None:
                worker = ProjectWorker(
                    self,
                    project.identity,
                    project.public_id,
                    self._runner,
                )
                self._workers[project.identity] = worker
            worker.submit(operation)
            transfer_secret_ownership()
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

    def _consume_admin_authorization_locked(
        self,
        authorization: AdminAuthorization | None,
        *,
        project: CanonicalProject,
        lease: ProjectLease,
        command_path: str,
    ) -> float:
        if authorization is not None:
            already_consumed = authorization.consumed
            authorization.consumed = True
        else:
            already_consumed = False
        valid = (
            authorization is not None
            and not already_consumed
            and time.monotonic() <= authorization.expires_at_monotonic
            and authorization.project_identity == project.identity
            and authorization.database_identity == lease.database_identity
            and authorization.lease_epoch == lease.epoch
        )
        if not valid:
            raise AppError(
                "admin commands require fresh authentication for the active lease",
                code="operator_admin_auth_required",
                details={"command": command_path},
                retryable=False,
            )
        return authorization.expires_at_monotonic

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
        with self._transition_condition:
            while self._close_in_progress:
                self._transition_condition.wait()
            if self._close_terminal_error is not None:
                raise self._close_terminal_error
            if self._close_complete.is_set() and not self._pending_owner_releases:
                return
            self._close_in_progress = True
            self._close_complete.clear()
            prepare_shutdown = not self._close_prepared
            if not self._closed.is_set():
                self._closed.set()
        first_error: BaseException | None = None
        try:
            if prepare_shutdown:
                with self._transition_condition:
                    while self._active_project_transitions:
                        self._transition_condition.wait()
                    for project_id in list(self._leases):
                        self._revoke_lease_locked(
                            project_id,
                            reason="operator broker stopped",
                        )
                    for worker in self._workers.values():
                        worker.stop()
                    self._close_prepared = True
            self._release_pending_owners()
        except BaseException as exc:
            first_error = exc
        finally:
            with self._transition_condition:
                if first_error is None:
                    self._close_complete.set()
                elif not self._close_prepared:
                    # Retrying partially applied lease/worker transitions would
                    # be unsafe. Repeated callers receive the original failure.
                    self._close_terminal_error = first_error
                self._close_in_progress = False
                self._transition_condition.notify_all()
        if first_error is not None:
            raise first_error

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

    def _auth_backoff_locked(
        self,
        project_identity: str,
        data_root: str,
    ) -> AuthAttemptBackoff:
        backoff = self._auth_backoffs.get(project_identity)
        if backoff is None:
            backoff = AuthAttemptBackoff(
                str(resolve_config_root(data_root) / AUTH_BACKOFF_FILENAME)
            )
            self._auth_backoffs[project_identity] = backoff
        self._auth_backoffs.move_to_end(project_identity)
        while len(self._auth_backoffs) > MAX_CACHED_AUTH_BACKOFFS:
            self._auth_backoffs.popitem(last=False)
        return backoff

    def _finish_operation_locked(
        self,
        operation: Operation,
        state: str,
        result: OperationResult | None,
    ) -> None:
        operation.state = state
        operation.output_error = None
        operation.retained_result_bytes = 0
        if result is not None:
            operation.retained_result_bytes = _result_bytes(result)
            if _result_exceeds_protocol_frame(result):
                operation.output_error = {
                    "code": "operator_result_too_large",
                    "message": (
                        "The operation finished, but its output exceeded the "
                        "broker response limit and was not retained."
                    ),
                    "hint": (
                        "Narrow the request or use a file export; reconcile any "
                        "mutation before retrying."
                    ),
                    "retryable": False,
                }
                result = OperationResult(result.exit_code, "", "")
                operation.retained_result_bytes = 0
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
        """Detach a lease while locked; owner handles are released later."""

        lease = self._leases.pop(project_id, None)
        if lease is None:
            return
        worker = self._workers.pop(project_id, None)
        if worker is not None:
            worker.drain()
            worker.stop()
        # A worker removes an operation from its deque before it enters the
        # project transition gate. Scan retained operations as well as draining
        # the deque so expiry cannot strand that popped-but-not-running work and
        # let it dispatch under a later lease.
        for operation in list(self._operations.values()):
            if operation.project_identity != project_id:
                continue
            if operation.state != "queued":
                continue
            operation.cancellation_requested = True
            self._finish_operation_locked(
                operation,
                "cancelled",
                OperationResult(1, "", "operator lease ended\n"),
            )
        _wipe(lease.passphrase)
        self._pending_owner_releases.append((project_id, lease.owner))
        for alias, identity in list(self._lease_aliases.items()):
            if identity == project_id:
                self._lease_aliases.pop(alias, None)

    def _release_pending_owners(
        self,
        project_identity: str | None = None,
    ) -> None:
        """Release detached owner handles without holding the service lock."""

        with self._lock:
            selected: list[tuple[str, ProjectOwnerLease]] = []
            retained: deque[tuple[str, ProjectOwnerLease]] = deque()
            while self._pending_owner_releases:
                identity, owner = self._pending_owner_releases.popleft()
                if project_identity is None or identity == project_identity:
                    selected.append((identity, owner))
                else:
                    retained.append((identity, owner))
            self._pending_owner_releases = retained
        first_error: Exception | None = None
        for identity, owner in selected:
            try:
                owner.release()
            except Exception as exc:
                if first_error is None:
                    first_error = exc
                with self._lock:
                    self._pending_owner_releases.append((identity, owner))
        if first_error is not None:
            raise first_error

    def _revoke_lease_locked(self, project_id: str, *, reason: str) -> None:
        lease = self._leases.get(project_id)
        if lease is None:
            return
        lease.revoked = True
        worker = self._workers.get(project_id)
        if worker is not None:
            worker.drain()
        for operation in list(self._operations.values()):
            if operation.project_identity != project_id:
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

    def _refresh_scope(
        self,
        project_identity: str,
        lease: ProjectLease,
    ) -> None:
        """Refresh context under the project gate, never the global lock."""

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
            with self._lock:
                if self._leases.get(project_identity) is lease:
                    self._revoke_lease_locked(
                        project_identity,
                        reason="operator scope could not be refreshed",
                    )
            return
        with self._lock:
            if self._leases.get(project_identity) is not lease or lease.revoked:
                return
            lease.workspace = (
                str(context.get("workspace_id"))
                if context.get("workspace_id")
                else None
            )
            lease.profile = (
                str(context.get("profile_id"))
                if context.get("profile_id")
                else None
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
        retained_bytes = sum(
            self._operations[operation_id].retained_result_bytes
            for operation_id in terminal_ids
        )
        while terminal_ids and (
            len(terminal_ids) > MAX_RETAINED_RESULTS
            or (
                retained_bytes > MAX_RETAINED_RESULT_BYTES
                and len(terminal_ids) > 1
            )
        ):
            removable = terminal_ids.pop(0)
            operation = self._operations.pop(removable, None)
            if operation is not None:
                retained_bytes = max(
                    0,
                    retained_bytes - operation.retained_result_bytes,
                )
                self._operation_tombstones[removable] = (
                    operation.project_id,
                    _operation_request_fingerprint(operation.argv),
                )
                self._operation_tombstones.move_to_end(removable)
        while len(self._operation_tombstones) > MAX_RETAINED_OPERATION_TOMBSTONES:
            self._operation_tombstones.popitem(last=False)

    def _expire_leases(self) -> None:
        while not self._closed.wait(1.0):
            with self._lock:
                expired = [
                    project_id
                    for project_id, lease in self._leases.items()
                    if lease.expired()
                ]
                pending = [
                    project_id
                    for project_id, _owner in self._pending_owner_releases
                ]
            for project_id in dict.fromkeys((*expired, *pending)):
                public_project_id = project_id[:16]
                try:
                    with self._project_transition(project_id, allow_closed=True):
                        with self._lock:
                            lease = self._leases.get(project_id)
                            if lease is not None:
                                public_project_id = lease.project.public_id
                            if lease is not None and lease.expired():
                                lease.revoked = True
                                _LOGGER.info(
                                    "operator lease expired",
                                    extra={
                                        "kb_fields": {
                                            "project": lease.project.public_id,
                                            "running_operations_finishing": (
                                                lease.running_operations
                                            ),
                                        }
                                    },
                                )
                                self._revoke_lease_locked(
                                    project_id,
                                    reason="operator lease expired",
                                )
                        try:
                            self._release_pending_owners(project_id)
                        except Exception:
                            _LOGGER.error(
                                "operator lease expiry cleanup failed",
                                extra={
                                    "kb_fields": {
                                        "project": public_project_id,
                                    }
                                },
                            )
                except Exception:
                    # Project-transition cleanup retries a failed owner release.
                    # Contain that retry too so one broken handle cannot kill the
                    # sole janitor and leave unrelated passphrases past expiry.
                    _LOGGER.error(
                        "operator lease expiry cleanup failed",
                        extra={
                            "kb_fields": {
                                "project": public_project_id,
                            }
                        },
                    )


def _classify_argv(argv: list[str]) -> tuple[str, Capability]:
    _args, path, capability = _parse_argv(argv)
    return path, capability


def _broker_stopped_error() -> AppError:
    return AppError(
        "the operator broker is stopping",
        code="operator_broker_stopped",
        retryable=True,
    )


def _result_bytes(result: OperationResult) -> int:
    return len(result.stdout.encode("utf-8")) + len(result.stderr.encode("utf-8"))


def _result_exceeds_protocol_frame(result: OperationResult) -> bool:
    encoded = json.dumps(
        {"stderr": result.stderr, "stdout": result.stdout},
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return len(encoded) > MAX_JSON_FRAME - _OPERATION_STATUS_FRAME_HEADROOM


def _operation_request_fingerprint(argv: list[str]) -> str:
    encoded = json.dumps(
        argv,
        ensure_ascii=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _require_explicit_scope(
    parsed: object,
    command_path: str,
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
    try:
        capability = cli_capability(path)
    except KeyError as exc:
        raise AppError(
            "the broker received an unclassified command",
            code="operator_unclassified_command",
            hint="Upgrade Kassiber or classify the command before brokered use.",
            details={"command": path},
            retryable=False,
        ) from exc
    return args, path, capability


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


def _wipe_secret_arguments(arguments: dict[str, bytearray]) -> None:
    for value in arguments.values():
        _wipe(value)
    arguments.clear()
