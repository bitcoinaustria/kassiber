from __future__ import annotations

import os
import json
import logging
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

import kassiber.cli.main as cli_main_module
from kassiber.db import (
    DATABASE_INSTANCE_ID_SETTING,
    database_instance_id,
    open_db,
    resolve_database_path,
)
from kassiber.errors import AppError
from kassiber.operator import runner as operator_runner
from kassiber.operator.client import (
    FROZEN_BROKER_STARTUP_TIMEOUT_SECONDS,
    SOURCE_BROKER_STARTUP_TIMEOUT_SECONDS,
    WINDOWS_FROZEN_BROKER_STARTUP_TIMEOUT_SECONDS,
    BrokerClient,
    _broker_startup_timeout_seconds,
    parse_duration,
    prepare_arguments,
    wipe_prepared,
)
from kassiber.operator.launcher import (
    broker_server_command,
    cli_child_command,
    prepare_independent_child_environment,
)
from kassiber.operator.project import canonical_project
from kassiber.operator.protocol import MAX_JSON_FRAME
from kassiber.command_capabilities import Capability
from kassiber.operator.service import (
    MAX_CACHED_AUTH_BACKOFFS,
    MAX_RETAINED_RESULTS,
    Operation,
    OperationResult,
    OperatorService,
    ProjectLease,
    ProjectWorker,
)
from kassiber.operator.runner import run_cli_operation
from kassiber.log_ring import LogRing, RingHandler
from kassiber.secrets.migration import create_empty_encrypted_database
from kassiber.secrets.sqlcipher import sqlcipher_available


class _Connection:
    def close(self) -> None:
        pass


def _scoped(command: str) -> list[str]:
    return [
        command,
        "--workspace",
        "workspace-a",
        "--profile",
        "book-a",
    ]


class OperatorServiceTest(unittest.TestCase):
    def test_native_auth_restart_requires_a_fully_idle_broker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.prepare_idle_restart()
                self.assertEqual(raised.exception.code, "operator_broker_busy")
                service.lock(tmp)
                self.assertEqual(
                    service.prepare_idle_restart()["restart"],
                    "accepted",
                )
            finally:
                service.close()

    def test_native_unlock_rejects_unexpected_database_identity(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                with self.assertRaises(AppError) as raised:
                    service.unlock(
                        tmp,
                        bytearray(b"passphrase"),
                        duration_seconds=None,
                        authentication_method="touch_id",
                        expected_database_identity="f" * 32,
                    )

                self.assertEqual(raised.exception.code, "operator_project_replaced")
                self.assertEqual(service._leases, {})
            finally:
                service.close()

    def test_native_unlock_rejects_project_replacement_before_open(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ) as open_database:
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                with self.assertRaises(AppError) as raised:
                    service.unlock(
                        tmp,
                        bytearray(b"passphrase"),
                        duration_seconds=None,
                        authentication_method="touch_id",
                        expected_project_identity="f" * 64,
                    )

                self.assertEqual(raised.exception.code, "operator_project_replaced")
                open_database.assert_not_called()
                self.assertEqual(service._leases, {})
            finally:
                service.close()

    def test_admin_cannot_be_granted_as_a_standing_lease(self) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        try:
            with self.assertRaises(AppError) as raised:
                service.unlock(
                    "/not-opened",
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                    capability=Capability.ADMIN,
                )
            self.assertEqual(
                raised.exception.code,
                "operator_invalid_lease_capability",
            )
            self.assertEqual(service._leases, {})
        finally:
            service.close()

    def test_auth_backoff_cache_is_bounded_and_lru(self) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        try:
            with service._lock:
                for index in range(MAX_CACHED_AUTH_BACKOFFS + 1):
                    service._auth_backoff_locked(
                        f"project-{index}",
                        f"/not-opened/project-{index}",
                    )
            self.assertEqual(
                len(service._auth_backoffs),
                MAX_CACHED_AUTH_BACKOFFS,
            )
            self.assertNotIn("project-0", service._auth_backoffs)
            self.assertIn(
                f"project-{MAX_CACHED_AUTH_BACKOFFS}",
                service._auth_backoffs,
            )
        finally:
            service.close()

    def test_secret_buffers_are_omitted_from_internal_representations(self) -> None:
        operation_secret = bytearray(b"operation-repr-secret")
        lease_secret = bytearray(b"lease-repr-secret")
        operation = Operation(
            id="operation",
            generation="generation",
            project_id="public-project",
            project_identity="project-identity",
            database_identity="database-identity",
            data_root="/redacted",
            argv=["status"],
            command_path="status",
            capability=Capability.READ,
            secret_arguments={"passphrase": operation_secret},
        )
        lease = ProjectLease(
            data_root="/redacted",
            project=mock.Mock(),
            database_identity="database-identity",
            passphrase=lease_secret,
            capability=Capability.READ,
            owner=mock.Mock(),
            unlocked_at="2026-01-01T00:00:00Z",
            expires_at_monotonic=None,
            duration_seconds=None,
            authentication_method="password",
            expires_at=None,
        )
        try:
            self.assertNotIn("operation-repr-secret", repr(operation))
            self.assertNotIn("lease-repr-secret", repr(lease))
        finally:
            operation_secret[:] = b"\0" * len(operation_secret)
            lease_secret[:] = b"\0" * len(lease_secret)

    def test_unlock_retains_owner_until_failed_verification_close_retries(self) -> None:
        connection = _Connection()
        connection.close = mock.Mock(
            side_effect=[OSError("close failed"), OSError("still failed"), None]
        )
        owner = mock.Mock()
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=connection,
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService("generation", lambda *_args: mock.Mock())
            try:
                with self.assertRaisesRegex(OSError, "still failed"):
                    service.unlock(
                        tmp,
                        bytearray(b"passphrase"),
                        duration_seconds=None,
                    )

                owner.release.assert_not_called()
                self.assertTrue(service._pending_owner_releases)

                service._release_pending_owners()

                self.assertEqual(connection.close.call_count, 3)
                owner.release.assert_called_once_with()
                self.assertFalse(service._pending_owner_releases)
            finally:
                service.close()

    def test_unlock_retains_owner_when_failed_open_release_needs_retry(self) -> None:
        owner = mock.Mock()
        owner.release.side_effect = [
            OSError("release failed"),
            OSError("still failed"),
            None,
        ]
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            side_effect=AppError("wrong passphrase", code="unlock_failed"),
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService("generation", lambda *_args: mock.Mock())
            try:
                with self.assertRaisesRegex(OSError, "still failed"):
                    service.unlock(
                        tmp,
                        bytearray(b"wrong-passphrase"),
                        duration_seconds=None,
                    )

                self.assertEqual(owner.release.call_count, 2)
                self.assertTrue(service._pending_owner_releases)

                service._release_pending_owners()

                self.assertEqual(owner.release.call_count, 3)
                self.assertFalse(service._pending_owner_releases)
            finally:
                service.close()

    def test_fresh_auth_retains_owner_until_failed_connection_close_retries(self) -> None:
        connection = _Connection()
        connection.close = mock.Mock(
            side_effect=[OSError("close failed"), OSError("still failed"), None]
        )
        owner = mock.Mock()
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=connection,
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService("generation", lambda *_args: mock.Mock())
            try:
                with self.assertRaisesRegex(OSError, "still failed"):
                    service.authenticate_database(
                        tmp,
                        bytearray(b"passphrase"),
                        scope="operator_mode",
                        require_lease=False,
                    )

                owner.release.assert_not_called()
                self.assertTrue(service._pending_owner_releases)

                service._release_pending_owners()

                self.assertEqual(connection.close.call_count, 3)
                owner.release.assert_called_once_with()
                self.assertFalse(service._pending_owner_releases)
            finally:
                service.close()

    def test_lifecycle_telemetry_is_bounded_ram_only_and_public_safe(self) -> None:
        ring = LogRing(max_records=3, max_bytes=4096)
        logger = logging.getLogger("kassiber.operator")
        previous_handlers = list(logger.handlers)
        previous_level = logger.level
        previous_propagate = logger.propagate
        logger.handlers = [RingHandler(ring)]
        logger.setLevel(logging.INFO)
        logger.propagate = False
        try:
            with tempfile.TemporaryDirectory() as tmp, mock.patch(
                "kassiber.operator.service.open_db", return_value=_Connection()
            ):
                service = OperatorService(
                    "generation",
                    lambda *_args: OperationResult(0, "", ""),
                )
                try:
                    service.unlock(
                        tmp,
                        bytearray(b"blinding_key=private-value"),
                        duration_seconds=None,
                    )
                    accepted = service.submit(tmp, ["status"])
                    self._wait_terminal(service, accepted["operation_id"])
                    service.lock(tmp)
                    records = ring.snapshot(limit=10)["records"]
                    self.assertLessEqual(len(records), 3)
                    rendered = repr(records)
                    self.assertNotIn(tmp, rendered)
                    self.assertNotIn("private-value", rendered)
                    self.assertTrue(
                        any("operator operation" in record["msg"] for record in records)
                    )
                finally:
                    service.close()
        finally:
            logger.handlers = previous_handlers
            logger.setLevel(previous_level)
            logger.propagate = previous_propagate

    def test_fresh_database_authentication_does_not_require_a_lease(self) -> None:
        owner = mock.Mock()
        continuation_ran = False

        def continuation(_database_identity: str) -> str:
            nonlocal continuation_ran
            continuation_ran = True
            owner.release.assert_not_called()
            return "configured"

        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as runtime, mock.patch.dict(
            os.environ,
            {"KASSIBER_OPERATOR_RUNTIME_DIR": runtime},
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ), mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ):
            os.chmod(runtime, 0o700)
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                result = service.authenticate_database(
                    tmp,
                    bytearray(b"fresh-passphrase"),
                    scope="operator_mode",
                    require_lease=False,
                    continuation=continuation,
                )
                self.assertEqual(result, "configured")
                self.assertTrue(continuation_ran)
                self.assertEqual(service.status(tmp)["lease"], "locked")
                owner.release.assert_called_once_with()
            finally:
                service.close()

    def test_fresh_authentication_releases_owner_when_backoff_rejects(self) -> None:
        owner = mock.Mock()
        backoff = mock.Mock()
        backoff.check.side_effect = AppError(
            "try again later",
            code="authentication_rate_limited",
            retryable=True,
        )

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ), mock.patch(
            "kassiber.operator.service.AuthAttemptBackoff",
            return_value=backoff,
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                with self.assertRaises(AppError) as raised:
                    service.authenticate_database(
                        tmp,
                        bytearray(b"fresh-passphrase"),
                        scope="operator_mode",
                        require_lease=False,
                    )
                self.assertEqual(
                    raised.exception.code,
                    "authentication_rate_limited",
                )
                owner.release.assert_called_once_with()
            finally:
                service.close()

    @unittest.skipIf(os.name == "nt", "POSIX symlink test")
    def test_mode_authentication_and_continuation_use_canonical_root(self) -> None:
        owner = mock.Mock()
        with tempfile.TemporaryDirectory() as root, tempfile.TemporaryDirectory() as parent:
            alias = Path(parent) / "project"
            alias.symlink_to(root, target_is_directory=True)
            with mock.patch(
                "kassiber.operator.service.acquire_project_ownership",
                return_value=owner,
            ), mock.patch(
                "kassiber.operator.service.open_db",
                return_value=_Connection(),
            ) as open_database, mock.patch(
                "kassiber.operator.service.set_unlock_mode",
                return_value="manual",
            ) as set_mode:
                service = OperatorService(
                    "generation",
                    lambda *_args: OperationResult(0, "", ""),
                )
                try:
                    result = service.set_mode_authenticated(
                        str(alias),
                        bytearray(b"passphrase"),
                        "manual",
                    )
                    canonical_root = str(Path(root).resolve())
                    self.assertEqual(result["mode"], "manual")
                    self.assertEqual(open_database.call_args.args[0], canonical_root)
                    project_identity = canonical_project(canonical_root).identity
                    set_mode.assert_called_once_with(
                        canonical_root,
                        "manual",
                        database_identity=project_identity[:32],
                        expected_project_identity=project_identity,
                    )
                    owner.release.assert_called_once_with()
                finally:
                    service.close()

    def test_project_worker_serializes_operations_and_retains_results(self) -> None:
        started: list[str] = []
        first_started = threading.Event()
        release_first = threading.Event()

        def runner(operation, _passphrase):
            started.append(operation.id)
            if len(started) == 1:
                first_started.set()
                release_first.wait(2)
            return OperationResult(0, operation.command_path + "\n", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(
                    tmp,
                    bytearray(b"correct horse battery staple"),
                    duration_seconds=None,
                )
                first = service.submit(tmp, ["status"])
                self.assertTrue(first_started.wait(1))
                second = service.submit(tmp, _scoped("health"))
                self.assertEqual(service.operation_status(second["operation_id"])["state"], "queued")
                release_first.set()
                first_result = self._wait(service, first["operation_id"])
                second_result = self._wait(service, second["operation_id"])
                self.assertEqual(first_result["state"], "completed")
                self.assertEqual(first_result["stdout"], "status\n")
                self.assertEqual(second_result["state"], "completed")
                self.assertEqual(started, [first["operation_id"], second["operation_id"]])
            finally:
                release_first.set()
                service.close()

    def test_project_worker_is_created_once_and_stops_with_the_service(self) -> None:
        workers: list[ProjectWorker] = []

        def create_worker(*args, **kwargs):
            worker = ProjectWorker(*args, **kwargs)
            workers.append(worker)
            return worker

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch(
            "kassiber.operator.service.ProjectWorker",
            side_effect=create_worker,
        ) as worker_factory:
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                operation_ids = [
                    service.submit(
                        tmp,
                        [command] if command == "status" else _scoped(command),
                    )["operation_id"]
                    for command in ("status", "health", "next-actions")
                ]
                for operation_id in operation_ids:
                    self.assertEqual(
                        self._wait_terminal(service, operation_id)["state"],
                        "completed",
                    )
                service.unlock(
                    tmp,
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                )
                self.assertEqual(worker_factory.call_count, 1)
                self.assertEqual(len(workers), 1)
                self.assertTrue(workers[0]._thread.is_alive())
            finally:
                service.close()
            workers[0]._thread.join(1)
            self.assertFalse(workers[0]._thread.is_alive())

    def test_admin_never_inherits_standing_lease(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, ["secrets", "verify"])
                self.assertEqual(raised.exception.code, "operator_admin_auth_required")
            finally:
                service.close()

    def test_admin_authorization_is_single_use_and_bound_to_lease_epoch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(
                    tmp,
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                )
                stale = service.verify_admin(
                    tmp,
                    bytearray(b"passphrase"),
                )
                service.lock(tmp)
                service.unlock(
                    tmp,
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                )
                with self.assertRaises(AppError) as stale_error:
                    service.submit(
                        tmp,
                        ["secrets", "verify"],
                        admin_authorization=stale,
                    )
                self.assertEqual(
                    stale_error.exception.code,
                    "operator_admin_auth_required",
                )

                fresh = service.verify_admin(
                    tmp,
                    bytearray(b"passphrase"),
                )
                accepted = service.submit(
                    tmp,
                    ["secrets", "verify"],
                    admin_authorization=fresh,
                )
                self.assertEqual(
                    self._wait_terminal(service, accepted["operation_id"])["state"],
                    "completed",
                )
                with self.assertRaises(AppError) as reused_error:
                    service.submit(
                        tmp,
                        ["secrets", "verify"],
                        admin_authorization=fresh,
                    )
                self.assertEqual(
                    reused_error.exception.code,
                    "operator_admin_auth_required",
                )
            finally:
                service.close()

    def test_rejected_submissions_wipe_every_staged_secret(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )

            def assert_rejected_and_wiped(
                argv: list[str],
                expected_code: str,
                *,
                operation_id: str | None = None,
            ) -> None:
                secret = bytearray(b"staged-secret")
                arguments = {"broker-secret-test": secret}
                with self.assertRaises(AppError) as raised:
                    service.submit(
                        tmp,
                        argv,
                        operation_id=operation_id,
                        secret_arguments=arguments,
                    )
                self.assertEqual(raised.exception.code, expected_code)
                self.assertEqual(set(secret), {0})
                self.assertEqual(arguments, {})

            try:
                assert_rejected_and_wiped(
                    ["--definitely-invalid"],
                    "operator_invalid_command",
                )
                assert_rejected_and_wiped(
                    ["operator", "status"],
                    "operator_command_not_brokerable",
                )

                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                assert_rejected_and_wiped(
                    ["transactions", "list"],
                    "operator_scope_required",
                )
                assert_rejected_and_wiped(
                    ["health"],
                    "operator_scope_required",
                )
                assert_rejected_and_wiped(
                    ["status"],
                    "operator_protocol_error",
                    operation_id="invalid-operation-id",
                )
                assert_rejected_and_wiped(
                    ["secrets", "verify"],
                    "operator_admin_auth_required",
                )

                lease = next(iter(service._leases.values()))
                lease.capability = Capability.READ
                assert_rejected_and_wiped(
                    [
                        "journals",
                        "process",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-a",
                    ],
                    "operator_capability_denied",
                )
            finally:
                service.close()

    def test_unclassified_command_is_a_public_safe_terminal_rejection(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.cli_capability",
            side_effect=KeyError("private registry detail"),
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            secret = bytearray(b"staged-secret")
            try:
                with self.assertRaises(AppError) as raised:
                    service.submit(
                        tmp,
                        ["status"],
                        secret_arguments={"broker-secret-test": secret},
                    )
                self.assertEqual(
                    raised.exception.code,
                    "operator_unclassified_command",
                )
                self.assertFalse(raised.exception.retryable)
                self.assertNotIn("private registry detail", str(raised.exception))
                self.assertEqual(set(secret), {0})
            finally:
                service.close()

    def test_generation_change_reports_result_unknown(self) -> None:
        service = OperatorService(
            "new-generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        try:
            status = service.operation_status("old-generation.client.123")
            self.assertEqual(status["state"], "result_unknown")
            self.assertEqual(status["reason"], "broker_generation_changed")
        finally:
            service.close()

    def test_lock_wipes_retained_passphrase(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
            retained = next(iter(service._leases.values())).passphrase
            service.lock(tmp)
            self.assertEqual(set(retained), {0})
            self.assertFalse(service._workers)
            service.close()

    def test_lock_cancels_queued_work_but_running_work_finishes(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            release.wait(2)
            return OperationResult(0, "finished\n", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                running = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                queued = service.submit(tmp, _scoped("health"))
                locked = service.lock(tmp)
                self.assertEqual(locked["running_operations_finishing"], 1)
                self.assertEqual(
                    service.operation_status(queued["operation_id"])["state"],
                    "cancelled",
                )
                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, _scoped("next-actions"))
                self.assertEqual(raised.exception.code, "interaction_required")
                release.set()
                self.assertEqual(
                    self._wait(service, running["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(
                    self._wait_terminal(service, queued["operation_id"])["state"],
                    "cancelled",
                )
            finally:
                release.set()
                service.close()

    def test_expiry_between_admission_and_dispatch_cancels_queued_work(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                running = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                queued = service.submit(tmp, _scoped("health"))
                lease = next(iter(service._leases.values()))
                lease.expires_at_monotonic = time.monotonic() - 1
                release.set()
                self.assertEqual(
                    self._wait_terminal(service, running["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(
                    self._wait_terminal(service, queued["operation_id"])["state"],
                    "cancelled",
                )
            finally:
                release.set()
                service.close()

    def test_janitor_survives_double_owner_release_failure(self) -> None:
        failed_twice = threading.Event()
        later_project_released = threading.Event()
        first_release_attempts = 0

        def release_first_owner() -> None:
            nonlocal first_release_attempts
            first_release_attempts += 1
            if first_release_attempts <= 2:
                if first_release_attempts == 2:
                    failed_twice.set()
                raise OSError("simulated owner release failure")

        first_owner = mock.Mock()
        first_owner.release.side_effect = release_first_owner
        second_owner = mock.Mock()
        second_owner.release.side_effect = later_project_released.set

        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            side_effect=[first_owner, second_owner],
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(
                    first,
                    bytearray(b"first-passphrase"),
                    duration_seconds=None,
                )
                service.unlock(
                    second,
                    bytearray(b"second-passphrase"),
                    duration_seconds=None,
                )
                first_project = canonical_project(first)
                second_project = canonical_project(second)
                with service._lock:
                    first_lease = service._leases[first_project.identity]
                    second_lease = service._leases[second_project.identity]
                    first_lease.expires_at_monotonic = time.monotonic() - 1

                self.assertTrue(failed_twice.wait(2.5))
                self.assertTrue(service._janitor.is_alive())

                with service._lock:
                    second_lease.expires_at_monotonic = time.monotonic() - 1
                self.assertTrue(later_project_released.wait(2.5))
                with service._lock:
                    self.assertNotIn(second_project.identity, service._leases)
                self.assertEqual(set(second_lease.passphrase), {0})
                self.assertTrue(service._janitor.is_alive())
            finally:
                service.close()

    def test_expired_worker_queue_cannot_revive_after_reunlock(self) -> None:
        started = threading.Event()
        release = threading.Event()
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            if operation.command_path == "status":
                started.set()
                release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"first-passphrase"), duration_seconds=None)
                running = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                stale = service.submit(tmp, _scoped("health"))
                lease = next(iter(service._leases.values()))
                lease.expires_at_monotonic = time.monotonic() - 1

                release.set()
                self.assertEqual(
                    self._wait_terminal(service, running["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(
                    self._wait_terminal(service, stale["operation_id"])["state"],
                    "cancelled",
                )

                service.unlock(
                    tmp,
                    bytearray(b"second-passphrase"),
                    duration_seconds=None,
                )
                fresh = service.submit(tmp, _scoped("health"))
                self.assertEqual(
                    self._wait_terminal(service, fresh["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(calls, ["status", "health"])
            finally:
                release.set()
                service.close()

    def test_expired_dequeued_operation_cannot_run_under_a_new_lease(self) -> None:
        dequeued = threading.Event()
        release_dequeued = threading.Event()
        calls: list[str] = []
        run_one = ProjectWorker._run_one

        def pause_after_dequeue(worker, operation):
            dequeued.set()
            if not release_dequeued.wait(2):
                raise AssertionError("timed out releasing dequeued operation")
            return run_one(worker, operation)

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch.object(ProjectWorker, "_run_one", pause_after_dequeue):
            service = OperatorService("generation", runner)
            try:
                service.unlock(
                    tmp,
                    bytearray(b"first-passphrase"),
                    duration_seconds=None,
                )
                stale = service.submit(tmp, ["status"])
                self.assertTrue(dequeued.wait(1))
                lease = next(iter(service._leases.values()))
                lease.expires_at_monotonic = time.monotonic() - 1

                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, _scoped("health"))
                self.assertEqual(raised.exception.code, "interaction_required")
                self.assertEqual(
                    service.operation_status(stale["operation_id"])["state"],
                    "cancelled",
                )

                service.unlock(
                    tmp,
                    bytearray(b"second-passphrase"),
                    duration_seconds=None,
                )
                release_dequeued.set()
                fresh = service.submit(tmp, _scoped("health"))
                self.assertEqual(
                    self._wait_terminal(service, fresh["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(calls, ["health"])
            finally:
                release_dequeued.set()
                service.close()

    def test_direct_reunlock_cancels_dequeued_work_from_expired_lease(self) -> None:
        dequeued = threading.Event()
        release_dequeued = threading.Event()
        calls: list[str] = []
        run_one = ProjectWorker._run_one

        def pause_after_dequeue(worker, operation):
            dequeued.set()
            if not release_dequeued.wait(2):
                raise AssertionError("timed out releasing dequeued operation")
            return run_one(worker, operation)

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch.object(ProjectWorker, "_run_one", pause_after_dequeue):
            service = OperatorService("generation", runner)
            try:
                service.unlock(
                    tmp,
                    bytearray(b"first-passphrase"),
                    duration_seconds=None,
                )
                stale = service.submit(tmp, ["status"])
                self.assertTrue(dequeued.wait(1))
                lease = next(iter(service._leases.values()))
                lease.expires_at_monotonic = time.monotonic() - 1

                service.unlock(
                    tmp,
                    bytearray(b"second-passphrase"),
                    duration_seconds=None,
                )
                self.assertEqual(
                    service.operation_status(stale["operation_id"])["state"],
                    "cancelled",
                )

                release_dequeued.set()
                fresh = service.submit(tmp, _scoped("health"))
                self.assertEqual(
                    self._wait_terminal(service, fresh["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(calls, ["health"])
            finally:
                release_dequeued.set()
                service.close()

    def test_queued_cancellation_is_immediately_terminal_and_wipes_secrets(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                secret = bytearray(b"token")
                queued = service.submit(
                    tmp,
                    ["status"],
                    secret_arguments={"broker-secret-test": secret},
                )
                cancelled = service.cancel(queued["operation_id"])
                self.assertEqual(cancelled["state"], "cancelled")
                self.assertEqual(set(secret), {0})
            finally:
                release.set()
                service.close()

    def test_running_cancel_is_truthfully_not_cancellable(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                accepted = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                cancelled = service.cancel(accepted["operation_id"])
                self.assertEqual(cancelled["state"], "running")
                self.assertEqual(cancelled["cancellation"], "not_cancellable")
            finally:
                release.set()
                service.close()

    def test_capability_is_rechecked_immediately_before_dispatch(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                running = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                queued = service.submit(
                    tmp,
                    [
                        "journals",
                        "process",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-a",
                    ],
                )
                lease = next(iter(service._leases.values()))
                lease.capability = Capability.READ
                release.set()
                self.assertEqual(
                    self._wait_terminal(service, running["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(
                    self._wait_terminal(service, queued["operation_id"])["state"],
                    "cancelled",
                )
            finally:
                release.set()
                service.close()

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
    def test_real_worker_child_uses_lease_without_secret_in_argv(self) -> None:
        passphrase = "correct horse battery staple"
        with tempfile.TemporaryDirectory() as tmp:
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            connection.close()
            service = OperatorService("generation", run_cli_operation)
            try:
                service.unlock(tmp, bytearray(passphrase.encode()), duration_seconds=None)
                accepted = service.submit(
                    tmp,
                    ["--data-root", tmp, "--machine", "status"],
                )
                completed = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(completed["state"], "completed")
                payload = json.loads(completed["stdout"])
                self.assertEqual(payload["kind"], "status")
                self.assertNotIn(passphrase, completed["stdout"])
                self.assertNotIn(passphrase, completed["stderr"])
            finally:
                service.close()

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
    @unittest.skipUnless(
        sys.platform.startswith("linux"),
        "Linux small-pipe regression",
    )
    def test_no_bootstrap_child_drains_lease_before_second_secret(self) -> None:
        import fcntl

        with tempfile.TemporaryDirectory() as tmp:
            passphrase = "x" * 5000
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            expected_database_identity = database_instance_id(connection)
            connection.close()
            project = canonical_project(tmp)
            marker = "broker-secret-backup"
            operation = Operation(
                id="generation.pipe-order",
                generation="generation",
                project_id=project.public_id,
                project_identity=project.identity,
                database_identity=expected_database_identity,
                data_root=str(project.database.parent),
                argv=[
                    "--data-root",
                    str(project.database.parent),
                    "--machine",
                    "backup",
                    "import",
                    str(Path(tmp) / "missing.kassiber"),
                    "--backup-passphrase-fd",
                    marker,
                ],
                command_path="backup.import",
                capability=Capability.ADMIN,
                secret_arguments={marker: bytearray(b"backup-secret")},
            )

            def small_secret_pipe() -> tuple[int, int, int]:
                read_fd, write_fd = os.pipe()
                fcntl.fcntl(write_fd, fcntl.F_SETPIPE_SZ, 4096)
                return read_fd, write_fd, read_fd

            outcome: list[OperationResult | BaseException] = []

            def execute() -> None:
                try:
                    outcome.append(
                        run_cli_operation(operation, bytearray(passphrase.encode()))
                    )
                except BaseException as exc:  # pragma: no cover - assertion below
                    outcome.append(exc)

            with mock.patch.object(
                operator_runner,
                "_secret_pipe",
                side_effect=small_secret_pipe,
            ):
                thread = threading.Thread(target=execute, daemon=True)
                thread.start()
                thread.join(5)
                if thread.is_alive():
                    if operation.process is not None:
                        operation.process.kill()
                    thread.join(2)
                    self.fail("operator child deadlocked on ordered secret pipes")

            self.assertEqual(len(outcome), 1)
            self.assertIsInstance(outcome[0], OperationResult)
            assert isinstance(outcome[0], OperationResult)
            self.assertEqual(outcome[0].exit_code, 1)
            self.assertIn("missing_backup", outcome[0].stdout)

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
    @unittest.skipUnless(
        sys.platform.startswith("linux"),
        "Linux small-pipe regression",
    )
    def test_worker_discards_caller_database_fd_before_lease_handoff(self) -> None:
        import fcntl

        with tempfile.TemporaryDirectory() as tmp:
            passphrase = "x" * 5000
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            expected_database_identity = database_instance_id(connection)
            connection.close()
            project = canonical_project(tmp)
            marker = "broker-secret-caller-database"
            operation = Operation(
                id="generation.database-pipe-order",
                generation="generation",
                project_id=project.public_id,
                project_identity=project.identity,
                database_identity=expected_database_identity,
                data_root=str(project.database.parent),
                argv=[
                    "--data-root",
                    str(project.database.parent),
                    "--machine",
                    "--db-passphrase-fd",
                    marker,
                    "status",
                ],
                command_path="status",
                capability=Capability.READ,
                secret_arguments={marker: bytearray(b"caller-secret")},
            )

            def small_secret_pipe() -> tuple[int, int, int]:
                read_fd, write_fd = os.pipe()
                fcntl.fcntl(write_fd, fcntl.F_SETPIPE_SZ, 4096)
                return read_fd, write_fd, read_fd

            outcome: list[OperationResult | BaseException] = []

            def execute() -> None:
                try:
                    outcome.append(
                        run_cli_operation(operation, bytearray(passphrase.encode()))
                    )
                except BaseException as exc:  # pragma: no cover - assertion below
                    outcome.append(exc)

            with mock.patch.object(
                operator_runner,
                "_secret_pipe",
                side_effect=small_secret_pipe,
            ):
                thread = threading.Thread(target=execute, daemon=True)
                thread.start()
                thread.join(5)
                if thread.is_alive():
                    if operation.process is not None:
                        operation.process.kill()
                    thread.join(2)
                    self.fail("operator child deadlocked on duplicate database pipes")

            self.assertEqual(len(outcome), 1)
            self.assertIsInstance(outcome[0], OperationResult)
            assert isinstance(outcome[0], OperationResult)
            self.assertEqual(outcome[0].exit_code, 0)
            self.assertEqual(json.loads(outcome[0].stdout)["kind"], "status")

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
    @unittest.skipUnless(
        sys.platform.startswith("linux"),
        "Linux small-pipe regression",
    )
    def test_worker_feeds_multiple_large_command_secrets_concurrently(self) -> None:
        import fcntl

        with tempfile.TemporaryDirectory() as tmp:
            passphrase = "correct horse battery staple"
            create_empty_encrypted_database(resolve_database_path(tmp), passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            expected_database_identity = database_instance_id(connection)
            connection.close()
            project = canonical_project(tmp)
            token_marker = "broker-secret-token"
            auth_marker = "broker-secret-auth-header"
            operation = Operation(
                id="generation.multi-secret-pipe-order",
                generation="generation",
                project_id=project.public_id,
                project_identity=project.identity,
                database_identity=expected_database_identity,
                data_root=str(project.database.parent),
                argv=[
                    "--data-root",
                    str(project.database.parent),
                    "--machine",
                    "backends",
                    "create",
                    "multi-secret",
                    "--kind",
                    "esplora",
                    "--url",
                    "http://127.0.0.1:3002",
                    "--token-fd",
                    token_marker,
                    "--auth-header-fd",
                    auth_marker,
                ],
                command_path="backends.create",
                capability=Capability.OPERATOR,
                secret_arguments={
                    token_marker: bytearray(b"t" * 5000),
                    auth_marker: bytearray(b"a" * 5000),
                },
            )

            def small_secret_pipe() -> tuple[int, int, int]:
                read_fd, write_fd = os.pipe()
                fcntl.fcntl(write_fd, fcntl.F_SETPIPE_SZ, 4096)
                return read_fd, write_fd, read_fd

            outcome: list[OperationResult | BaseException] = []

            def execute() -> None:
                try:
                    outcome.append(
                        run_cli_operation(operation, bytearray(passphrase.encode()))
                    )
                except BaseException as exc:  # pragma: no cover - assertion below
                    outcome.append(exc)

            with mock.patch.object(
                operator_runner,
                "_secret_pipe",
                side_effect=small_secret_pipe,
            ):
                thread = threading.Thread(target=execute, daemon=True)
                thread.start()
                thread.join(7)
                if thread.is_alive():
                    if operation.process is not None:
                        operation.process.kill()
                    thread.join(2)
                    self.fail("operator child deadlocked on multiple secret pipes")

            self.assertEqual(len(outcome), 1)
            self.assertIsInstance(outcome[0], OperationResult)
            assert isinstance(outcome[0], OperationResult)
            self.assertEqual(outcome[0].exit_code, 0, outcome[0])

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
    @unittest.skipIf(os.name == "nt", "POSIX fd regression")
    def test_no_bootstrap_child_binds_database_before_dispatch(self) -> None:
        passphrase = "correct horse battery staple"
        with tempfile.TemporaryDirectory() as tmp:
            db_path = resolve_database_path(tmp)
            create_empty_encrypted_database(db_path, passphrase)
            connection = open_db(tmp, passphrase=passphrase)
            expected_database_identity = database_instance_id(connection)
            inode_before = db_path.stat().st_ino
            connection.execute(
                "UPDATE settings SET value = ? WHERE key = ?",
                ("f" * 32, DATABASE_INSTANCE_ID_SETTING),
            )
            connection.commit()
            connection.close()
            self.assertEqual(db_path.stat().st_ino, inode_before)
            project = canonical_project(tmp)

            read_fd, write_fd = os.pipe()
            os.write(write_fd, passphrase.encode())
            os.close(write_fd)
            environment = {
                "KASSIBER_OPERATOR_DIRECT": "1",
                "KASSIBER_OPERATOR_CHILD": "1",
                "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY": project.identity,
                "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY": expected_database_identity,
            }
            try:
                with mock.patch.dict(os.environ, environment), mock.patch.object(
                    cli_main_module,
                    "dispatch",
                ) as dispatch, mock.patch.object(cli_main_module, "emit_error"):
                    exit_code = cli_main_module.main(
                        [
                            "--data-root",
                            tmp,
                            "--machine",
                            "--db-passphrase-fd",
                            str(read_fd),
                            "secrets",
                            "forget-unlock",
                        ]
                    )
            finally:
                try:
                    os.close(read_fd)
                except OSError:
                    pass

            self.assertEqual(exit_code, 1)
            dispatch.assert_not_called()

    @unittest.skipUnless(sqlcipher_available(), "SQLCipher is required")
    def test_real_brokered_passphrase_rotation_revokes_stale_lease(self) -> None:
        old_passphrase = "correct horse battery staple"
        new_passphrase = bytearray(b"new correct horse battery staple")
        with tempfile.TemporaryDirectory() as tmp:
            create_empty_encrypted_database(resolve_database_path(tmp), old_passphrase)
            connection = open_db(tmp, passphrase=old_passphrase)
            connection.close()
            service = OperatorService("generation", run_cli_operation)
            try:
                service.unlock(
                    tmp,
                    bytearray(old_passphrase.encode()),
                    duration_seconds=None,
                )
                authorization = service.verify_admin(
                    tmp,
                    bytearray(old_passphrase.encode()),
                )
                accepted = service.submit(
                    tmp,
                    [
                        "--machine",
                        "secrets",
                        "change-passphrase",
                        "--new-passphrase-fd",
                        "broker-secret-new-passphrase",
                    ],
                    secret_arguments={
                        "broker-secret-new-passphrase": new_passphrase,
                    },
                    admin_authorization=authorization,
                )
                completed = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(completed["state"], "completed", completed)
                self.assertEqual(service.status(tmp)["lease"], "locked")
                with self.assertRaises(AppError):
                    open_db(tmp, passphrase=old_passphrase)
                reopened = open_db(tmp, passphrase="new correct horse battery staple")
                reopened.close()
            finally:
                service.close()

    def test_different_projects_run_concurrently(self) -> None:
        both_started = threading.Event()
        release = threading.Event()
        starts: list[str] = []
        guard = threading.Lock()

        def runner(operation, _passphrase):
            with guard:
                starts.append(operation.project_id)
                if len(starts) == 2:
                    both_started.set()
            release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(first, bytearray(b"first-passphrase"), duration_seconds=None)
                service.unlock(second, bytearray(b"second-passphrase"), duration_seconds=None)
                one = service.submit(first, ["status"])
                two = service.submit(second, ["status"])
                self.assertTrue(both_started.wait(1))
                release.set()
                self.assertEqual(self._wait(service, one["operation_id"])["state"], "completed")
                self.assertEqual(self._wait(service, two["operation_id"])["state"], "completed")
                self.assertEqual(len(set(starts)), 2)
            finally:
                release.set()
                service.close()

    def test_explicit_book_scope_survives_queue_admission(self) -> None:
        seen: list[list[str]] = []

        def runner(operation, _passphrase):
            seen.append(operation.argv)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                accepted = service.submit(
                    tmp,
                    [
                        "transactions",
                        "list",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-b",
                    ],
                )
                self.assertEqual(
                    self._wait(service, accepted["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(
                    seen[0][:8],
                    [
                        "--data-root",
                        str(Path(tmp).resolve()),
                        "transactions",
                        "list",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-b",
                    ],
                )
            finally:
                service.close()

    def test_child_project_locator_cannot_borrow_another_projects_lease(self) -> None:
        called = False

        def runner(_operation, _passphrase):
            nonlocal called
            called = True
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(first, bytearray(b"shared-passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.submit(
                        first,
                        ["--data-root", second, "--machine", "status"],
                    )
                self.assertEqual(raised.exception.code, "operator_project_mismatch")
                self.assertFalse(called)
                self.assertNotIn(first, repr(raised.exception.details))
                self.assertNotIn(second, repr(raised.exception.details))
            finally:
                service.close()

    def test_operation_id_cannot_be_reused_across_projects(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(first, bytearray(b"first"), duration_seconds=None)
                service.unlock(second, bytearray(b"second"), duration_seconds=None)
                operation_id = "generation.client.fixed"
                service.submit(first, ["status"], operation_id=operation_id)
                with self.assertRaises(AppError) as raised:
                    service.submit(second, ["status"], operation_id=operation_id)
                self.assertEqual(
                    raised.exception.code,
                    "operator_operation_id_conflict",
                )
            finally:
                service.close()

    def test_replaced_database_path_keeps_prior_lease_revocable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            database = os.path.join(tmp, "kassiber.sqlite3")
            with open(database, "wb") as handle:
                handle.write(b"first")
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                retained = next(iter(service._leases.values())).passphrase
                os.replace(database, os.path.join(tmp, "old.sqlite3"))
                with open(database, "wb") as handle:
                    handle.write(b"replacement")
                status = service.status(tmp)
                self.assertTrue(status["project_file_changed"])
                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, ["status"])
                self.assertEqual(raised.exception.code, "operator_project_replaced")
                self.assertTrue(service.lock(tmp)["lease_existed"])
                self.assertEqual(set(retained), {0})
                self.assertFalse(service._workers)
            finally:
                service.close()

    def test_bounded_queue_rejects_before_false_acceptance(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            release.wait()
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                accepted = [service.submit(tmp, ["status"]) for _ in range(64)]
                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, ["status"])
                self.assertEqual(raised.exception.code, "operator_queue_full")
                self.assertTrue(all(item["state"] == "queued" for item in accepted))
                service.cancel(accepted[0]["operation_id"])
                replacement = service.submit(tmp, ["status"])
                self.assertEqual(replacement["state"], "queued")
            finally:
                release.set()
                service.close()

    def test_queued_operation_is_cancelled_if_database_identity_changes(self) -> None:
        started = threading.Event()
        release = threading.Event()
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.id)
            started.set()
            release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            database = resolve_database_path(tmp)
            database.write_bytes(b"original")
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                first = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                queued = service.submit(tmp, _scoped("health"))
                database.replace(database.with_suffix(".old"))
                database.write_bytes(b"replacement")
                release.set()
                self._wait_terminal(service, first["operation_id"])
                completed = self._wait_terminal(service, queued["operation_id"])
                self.assertEqual(completed["state"], "cancelled")
                self.assertEqual(calls, [first["operation_id"]])
            finally:
                release.set()
                service.close()

    def test_queued_admin_authentication_expires_before_dispatch(self) -> None:
        started = threading.Event()
        release = threading.Event()
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            if len(calls) == 1:
                started.set()
                release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                authorization = service.verify_admin(
                    tmp,
                    bytearray(b"passphrase"),
                )
                first = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                queued = service.submit(
                    tmp,
                    ["secrets", "verify"],
                    admin_authorization=authorization,
                )
                service._operations[
                    queued["operation_id"]
                ].admin_authorized_until_monotonic = time.monotonic() - 1
                release.set()
                self._wait_terminal(service, first["operation_id"])
                completed = self._wait_terminal(service, queued["operation_id"])
                self.assertEqual(completed["state"], "cancelled")
                self.assertEqual(calls, ["status"])
            finally:
                release.set()
                service.close()

    def test_queued_operation_cannot_cross_a_refreshed_lease_epoch(self) -> None:
        dequeued = threading.Event()
        release = threading.Event()
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"first-passphrase"), duration_seconds=None)
                worker = next(iter(service._workers.values()))
                original_run_one = worker._run_one

                def delayed_run(operation):
                    dequeued.set()
                    if not release.wait(2):
                        raise AssertionError("timed out waiting for lease refresh")
                    original_run_one(operation)

                worker._run_one = delayed_run  # type: ignore[method-assign]
                queued = service.submit(tmp, ["status"])
                self.assertTrue(dequeued.wait(1))

                service.unlock(
                    tmp,
                    bytearray(b"second-passphrase"),
                    duration_seconds=None,
                )
                release.set()
                completed = self._wait_terminal(service, queued["operation_id"])

                self.assertEqual(completed["state"], "cancelled")
                self.assertEqual(service.status(tmp)["lease"], "unlocked")
                replacement = service.submit(tmp, _scoped("health"))
                self.assertEqual(
                    self._wait_terminal(service, replacement["operation_id"])[
                        "state"
                    ],
                    "completed",
                )
                self.assertEqual(calls, ["health"])
            finally:
                release.set()
                service.close()

    def test_fresh_auth_never_opens_database_during_same_project_work(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            if not release.wait(2):
                raise AssertionError("timed out waiting to finish operation")
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ) as open_database:
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                open_database.reset_mock()

                with self.assertRaises(AppError) as admin_error:
                    service.verify_admin(tmp, bytearray(b"passphrase"))
                self.assertEqual(admin_error.exception.code, "operator_project_busy")

                with self.assertRaises(AppError) as mode_error:
                    service.authenticate_database(
                        tmp,
                        bytearray(b"passphrase"),
                        scope="operator_mode",
                        require_lease=False,
                    )
                self.assertEqual(mode_error.exception.code, "operator_project_busy")
                open_database.assert_not_called()
            finally:
                release.set()
                service.close()

    def test_admin_authentication_is_rechecked_after_owner_inheritance(self) -> None:
        inheritance_started = threading.Event()
        release_inheritance = threading.Event()
        inherited_owner = mock.Mock()
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(
                    tmp,
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                )
                authorization = service.verify_admin(
                    tmp,
                    bytearray(b"passphrase"),
                )
                lease = next(iter(service._leases.values()))

                def duplicate_owner():
                    inheritance_started.set()
                    if not release_inheritance.wait(2):
                        raise AssertionError("timed out waiting to inherit owner")
                    return inherited_owner

                with mock.patch.object(
                    lease.owner,
                    "duplicate_for_child",
                    side_effect=duplicate_owner,
                ):
                    queued = service.submit(
                        tmp,
                        ["secrets", "verify"],
                        admin_authorization=authorization,
                    )
                    self.assertTrue(inheritance_started.wait(1))
                    service._operations[
                        queued["operation_id"]
                    ].admin_authorized_until_monotonic = time.monotonic() - 1
                    release_inheritance.set()
                    completed = self._wait_terminal(
                        service,
                        queued["operation_id"],
                    )

                self.assertEqual(completed["state"], "cancelled")
                self.assertEqual(calls, [])
                inherited_owner.close.assert_called_once_with()
            finally:
                release_inheritance.set()
                service.close()

    def test_terminal_transition_prunes_retained_results(self) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        try:
            for index in range(MAX_RETAINED_RESULTS + 25):
                operation = Operation(
                    id=f"generation.test.{index}",
                    generation="generation",
                    project_id="project",
                    project_identity="identity",
                    database_identity="database-identity",
                    data_root="/unused",
                    argv=["status"],
                    command_path="status",
                    capability=Capability.READ,
                    secret_arguments={},
                    state="completed",
                )
                service._operations[operation.id] = operation
            operation.state = "queued"
            service._finish_operation_locked(
                operation,
                "completed",
                OperationResult(0, "", ""),
            )
            self.assertEqual(len(service._operations), MAX_RETAINED_RESULTS)
        finally:
            service.close()

    def test_oversized_result_has_a_small_truthful_status_contract(self) -> None:
        oversized = "x" * (MAX_JSON_FRAME + 1)

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, oversized, ""),
            )
            try:
                service.unlock(
                    tmp,
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                )
                accepted = service.submit(tmp, ["status"])
                completed = self._wait_terminal(
                    service,
                    accepted["operation_id"],
                )
                self.assertEqual(completed["state"], "completed")
                self.assertEqual(completed["exit_code"], 0)
                self.assertFalse(completed["output_available"])
                self.assertEqual(
                    completed["output_error"]["code"],
                    "operator_result_too_large",
                )
                self.assertNotIn("stdout", completed)
                self.assertNotIn("stderr", completed)
                retained = service._operations[accepted["operation_id"]]
                self.assertEqual(retained.result, OperationResult(0, "", ""))
                self.assertEqual(retained.retained_result_bytes, 0)
                encoded = json.dumps(
                    {"ok": True, "data": completed},
                    separators=(",", ":"),
                    sort_keys=True,
                ).encode("utf-8")
                self.assertLess(len(encoded), MAX_JSON_FRAME)
            finally:
                service.close()

    def test_result_byte_budget_evicts_to_bounded_fingerprint_tombstones(
        self,
    ) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        sensitive_argument = "private-nonsecret-argument-" + ("z" * 4096)
        try:
            with mock.patch(
                "kassiber.operator.service.MAX_RETAINED_RESULT_BYTES",
                20,
            ):
                operations = []
                for index in range(3):
                    operation = Operation(
                        id=f"generation.byte-budget.{index}",
                        generation="generation",
                        project_id="project",
                        project_identity="identity",
                        database_identity="database-identity",
                        data_root="/unused",
                        argv=["status", sensitive_argument, str(index)],
                        command_path="status",
                        capability=Capability.READ,
                        secret_arguments={},
                    )
                    service._operations[operation.id] = operation
                    service._finish_operation_locked(
                        operation,
                        "completed",
                        OperationResult(0, "twelve-bytes", ""),
                    )
                    operations.append(operation)

            self.assertEqual(list(service._operations), [operations[-1].id])
            tombstone = service._operation_tombstones[operations[0].id]
            self.assertEqual(tombstone[0], "project")
            self.assertEqual(len(tombstone[1]), 64)
            self.assertNotIn(sensitive_argument, repr(service._operation_tombstones))
        finally:
            service.close()

    def test_active_operations_do_not_evict_the_first_completed_result(self) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        try:
            for index in range(MAX_RETAINED_RESULTS + 25):
                queued = Operation(
                    id=f"generation.active.{index}",
                    generation="generation",
                    project_id="project",
                    project_identity="identity",
                    database_identity="database-identity",
                    data_root="/unused",
                    argv=["status"],
                    command_path="status",
                    capability=Capability.READ,
                    secret_arguments={},
                )
                service._operations[queued.id] = queued
            completed = Operation(
                id="generation.completed",
                generation="generation",
                project_id="project",
                project_identity="identity",
                database_identity="database-identity",
                data_root="/unused",
                argv=["status"],
                command_path="status",
                capability=Capability.READ,
                secret_arguments={},
            )
            service._operations[completed.id] = completed
            service._finish_operation_locked(
                completed,
                "completed",
                OperationResult(0, "done", ""),
            )
            self.assertIn(completed.id, service._operations)
            self.assertEqual(
                service.operation_status(completed.id)["state"],
                "completed",
            )
        finally:
            service.close()

    def test_terminal_retention_uses_completion_order(self) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        try:
            operations = []
            for index in range(MAX_RETAINED_RESULTS + 1):
                operation = Operation(
                    id=f"generation.completion-order.{index}",
                    generation="generation",
                    project_id="project",
                    project_identity="identity",
                    database_identity="database-identity",
                    data_root="/unused",
                    argv=["status"],
                    command_path="status",
                    capability=Capability.READ,
                    secret_arguments={},
                )
                service._operations[operation.id] = operation
                operations.append(operation)

            for operation in reversed(operations):
                service._finish_operation_locked(
                    operation,
                    "completed",
                    OperationResult(0, "", ""),
                )

            self.assertNotIn(operations[-1].id, service._operations)
            self.assertIn(operations[0].id, service._operations)
            self.assertEqual(len(service._operations), MAX_RETAINED_RESULTS)
        finally:
            service.close()

    def test_evicted_operation_id_is_not_reexecuted(self) -> None:
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch("kassiber.operator.service.MAX_RETAINED_RESULTS", 1):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                first = service.submit(tmp, ["status"])
                self._wait_terminal(service, first["operation_id"])
                second = service.submit(tmp, _scoped("health"))
                self._wait_terminal(service, second["operation_id"])
                replay = service.submit(
                    tmp,
                    ["status"],
                    operation_id=first["operation_id"],
                )
                self.assertEqual(replay["state"], "result_unknown")
                self.assertEqual(replay["reason"], "result_not_retained")
                self.assertEqual(calls, ["status", "health"])
            finally:
                service.close()

    def test_worker_crash_marks_result_unknown(self) -> None:
        def runner(_operation, _passphrase):
            raise RuntimeError("simulated crash after unknown commit point")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                accepted = service.submit(tmp, ["status"])
                status = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(status["state"], "result_unknown")
            finally:
                service.close()

    def test_owner_inheritance_failure_before_launch_is_failed(self) -> None:
        runner = mock.Mock(return_value=OperationResult(0, "", ""))
        owner = mock.Mock()
        owner.duplicate_for_child.side_effect = OSError(
            "passphrase=pre-launch-secret"
        )

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                accepted = service.submit(tmp, ["status"])
                terminal = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(terminal["state"], "failed")
                self.assertEqual(terminal["exit_code"], 1)
                self.assertNotIn("pre-launch-secret", terminal["stderr"])
                runner.assert_not_called()
            finally:
                service.close()

    def test_owner_cleanup_failure_preserves_result_and_revokes_lease(self) -> None:
        calls: list[str] = []

        def runner(operation, _passphrase):
            calls.append(operation.command_path)
            return OperationResult(0, "", "")

        first_child = mock.Mock(tokens=())
        first_child.close.side_effect = OSError("passphrase=cleanup-secret")
        second_child = mock.Mock(tokens=())
        owner = mock.Mock()
        owner.duplicate_for_child.side_effect = [first_child, second_child]

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                lease = next(iter(service._leases.values()))
                first = service.submit(tmp, ["status"])
                first_status = self._wait_terminal(service, first["operation_id"])
                self.assertEqual(first_status["state"], "completed")
                self.assertNotIn("cleanup-secret", first_status["stderr"])
                self.assertEqual(lease.running_operations, 0)
                self.assertEqual(set(lease.passphrase), {0})
                self.assertEqual(service.status(tmp)["lease"], "locked")

                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                second = service.submit(tmp, _scoped("health"))
                second_status = self._wait_terminal(service, second["operation_id"])
                self.assertEqual(second_status["state"], "completed")
                self.assertEqual(calls, ["status", "health"])

                service.lock(tmp)
            finally:
                service.close()

    def test_transient_owner_release_logs_preserved_terminal_state(self) -> None:
        started = threading.Event()
        release = threading.Event()
        recovered = threading.Event()
        records: list[logging.LogRecord] = []

        class CaptureHandler(logging.Handler):
            def emit(self, record: logging.LogRecord) -> None:
                records.append(record)
                if record.getMessage() == (
                    "operator worker recovered from an internal failure"
                ):
                    recovered.set()

        def runner(_operation, _passphrase):
            started.set()
            if not release.wait(2):
                raise AssertionError("timed out releasing operation")
            return OperationResult(0, "", "")

        child = mock.Mock(tokens=())
        owner = mock.Mock()
        owner.duplicate_for_child.return_value = child
        owner.release.side_effect = [OSError("transient close failure"), None]
        logger = logging.getLogger("kassiber.operator")
        previous_handlers = list(logger.handlers)
        previous_level = logger.level
        previous_propagate = logger.propagate
        logger.handlers = [CaptureHandler()]
        logger.setLevel(logging.INFO)
        logger.propagate = False
        try:
            with tempfile.TemporaryDirectory() as tmp, mock.patch(
                "kassiber.operator.service.open_db", return_value=_Connection()
            ), mock.patch(
                "kassiber.operator.service.acquire_project_ownership",
                return_value=owner,
            ):
                service = OperatorService("generation", runner)
                try:
                    service.unlock(
                        tmp,
                        bytearray(b"passphrase"),
                        duration_seconds=None,
                    )
                    accepted = service.submit(tmp, ["status"])
                    self.assertTrue(started.wait(1))
                    service.lock(tmp)
                    release.set()
                    terminal = self._wait_terminal(
                        service,
                        accepted["operation_id"],
                    )
                    self.assertEqual(terminal["state"], "completed")
                    self.assertTrue(recovered.wait(1))
                    recovery_record = next(
                        record
                        for record in records
                        if record.getMessage()
                        == "operator worker recovered from an internal failure"
                    )
                    self.assertEqual(
                        recovery_record.kb_fields["state"],
                        "completed",
                    )
                    self.assertEqual(owner.release.call_count, 2)
                finally:
                    release.set()
                    service.close()
        finally:
            logger.handlers = previous_handlers
            logger.setLevel(previous_level)
            logger.propagate = previous_propagate

    def test_runner_exception_is_secret_floor_redacted(self) -> None:
        def runner(_operation, _passphrase):
            raise RuntimeError("blinding_key=private-value passphrase=hunter2")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                accepted = service.submit(tmp, ["status"])
                status = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(status["state"], "result_unknown")
                self.assertNotIn("private-value", status["stderr"])
                self.assertNotIn("hunter2", status["stderr"])
            finally:
                service.close()

    def test_completed_child_stderr_is_secret_floor_redacted(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(
                    1,
                    "",
                    'warning {"blinding_key":"private-value"} token=btcpay-secret\n',
                ),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                accepted = service.submit(tmp, ["status"])
                status = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(status["state"], "failed")
                self.assertNotIn("private-value", status["stderr"])
                self.assertNotIn("btcpay-secret", status["stderr"])
                self.assertIn("[redacted]", status["stderr"])
            finally:
                service.close()

    def test_unproven_nonzero_mutation_is_unknown_but_read_failure_is_failed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(1, "", "child exited\n"),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                read = service.submit(tmp, ["status"])
                mutation = service.submit(
                    tmp,
                    [
                        "journals",
                        "process",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-a",
                    ],
                )
                self.assertEqual(
                    self._wait_terminal(service, read["operation_id"])["state"],
                    "failed",
                )
                self.assertEqual(
                    self._wait_terminal(service, mutation["operation_id"])["state"],
                    "result_unknown",
                )
            finally:
                service.close()

    def test_successful_passphrase_rotation_revokes_the_lease(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(tmp, bytearray(b"old-passphrase"), duration_seconds=None)
                authorization = service.verify_admin(
                    tmp,
                    bytearray(b"old-passphrase"),
                )
                accepted = service.submit(
                    tmp,
                    [
                        "secrets",
                        "change-passphrase",
                        "--new-passphrase-fd",
                        "broker-secret-test",
                    ],
                    secret_arguments={
                        "broker-secret-test": bytearray(b"new-passphrase")
                    },
                    admin_authorization=authorization,
                )
                self.assertEqual(
                    self._wait_terminal(service, accepted["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(service.status(tmp)["lease"], "locked")
            finally:
                service.close()

    def test_credential_mode_commands_revoke_lease_after_success(self) -> None:
        for command in ("remember-unlock", "forget-unlock"):
            with self.subTest(command=command), tempfile.TemporaryDirectory() as tmp:
                service = OperatorService(
                    "generation",
                    lambda *_args: OperationResult(0, "", ""),
                )
                with mock.patch(
                    "kassiber.operator.service.open_db",
                    return_value=_Connection(),
                ):
                    try:
                        service.unlock(
                            tmp,
                            bytearray(b"passphrase"),
                            duration_seconds=None,
                        )
                        retained = next(iter(service._leases.values())).passphrase
                        authorization = service.verify_admin(
                            tmp,
                            bytearray(b"passphrase"),
                        )
                        accepted = service.submit(
                            tmp,
                            ["secrets", command],
                            admin_authorization=authorization,
                        )
                        terminal = self._wait_terminal(
                            service,
                            accepted["operation_id"],
                        )
                        self.assertEqual(terminal["state"], "completed")
                        self.assertEqual(service.status(tmp)["lease"], "locked")
                        self.assertEqual(set(retained), {0})
                    finally:
                        service.close()

    def test_credential_mode_partial_outcomes_revoke_and_drain_lease(self) -> None:
        for command in ("remember-unlock", "forget-unlock"):
            with self.subTest(command=command), tempfile.TemporaryDirectory() as tmp:
                started = threading.Event()
                release = threading.Event()

                def runner(operation, _passphrase):
                    if operation.command_path == f"secrets.{command}":
                        started.set()
                        if not release.wait(2):
                            raise AssertionError("timed out releasing credential command")
                        return OperationResult(1, "", "partial credential update\n")
                    return OperationResult(0, "", "")

                service = OperatorService("generation", runner)
                with mock.patch(
                    "kassiber.operator.service.open_db",
                    return_value=_Connection(),
                ):
                    try:
                        service.unlock(
                            tmp,
                            bytearray(b"passphrase"),
                            duration_seconds=None,
                        )
                        retained = next(iter(service._leases.values())).passphrase
                        authorization = service.verify_admin(
                            tmp,
                            bytearray(b"passphrase"),
                        )
                        credential = service.submit(
                            tmp,
                            ["secrets", command],
                            admin_authorization=authorization,
                        )
                        self.assertTrue(started.wait(1))
                        queued = service.submit(tmp, ["status"])
                        release.set()

                        credential_terminal = self._wait_terminal(
                            service,
                            credential["operation_id"],
                        )
                        self.assertEqual(
                            credential_terminal["state"],
                            "result_unknown",
                        )
                        self.assertEqual(
                            self._wait_terminal(
                                service,
                                queued["operation_id"],
                            )["state"],
                            "cancelled",
                        )
                        self.assertEqual(service.status(tmp)["lease"], "locked")
                        self.assertEqual(set(retained), {0})
                    finally:
                        release.set()
                        service.close()

    def test_cross_project_backup_install_is_rejected_before_admission(self) -> None:
        called = False

        def runner(_operation, _passphrase):
            nonlocal called
            called = True
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as source, tempfile.TemporaryDirectory() as target, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            secret = bytearray(b"backup-passphrase")
            try:
                service.unlock(source, bytearray(b"passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.submit(
                        source,
                        [
                            "backup",
                            "import",
                            "archive.kassiber",
                            "--install",
                            "--target-data-root",
                            target,
                            "--backup-passphrase-fd",
                            "broker-secret-backup",
                        ],
                        secret_arguments={"broker-secret-backup": secret},
                    )
                self.assertEqual(
                    raised.exception.code,
                    "operator_command_not_brokerable",
                )
                self.assertEqual(set(secret), {0})
                self.assertFalse(called)
            finally:
                service.close()

    @unittest.skipIf(os.name == "nt", "POSIX symlink retarget test")
    def test_backup_install_does_not_admit_an_explicit_symlink_target(self) -> None:
        called = False

        def runner(_operation, _passphrase):
            nonlocal called
            called = True
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as source, tempfile.TemporaryDirectory() as parent, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            alias = Path(parent) / "target"
            alias.symlink_to(source, target_is_directory=True)
            service = OperatorService("generation", runner)
            try:
                service.unlock(source, bytearray(b"passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.submit(
                        source,
                        [
                            "backup",
                            "import",
                            "archive.kassiber",
                            "--install",
                            "--target-data-root",
                            str(alias),
                        ],
                    )
                self.assertEqual(
                    raised.exception.code,
                    "operator_command_not_brokerable",
                )
                self.assertFalse(called)
                self.assertFalse(service._operations)
            finally:
                service.close()

    @unittest.skipIf(os.name == "nt", "POSIX symlink retarget test")
    def test_queued_child_is_pinned_to_canonical_root_not_caller_alias(self) -> None:
        started = threading.Event()
        release = threading.Event()
        seen: list[tuple[str, list[str]]] = []

        def runner(operation, _passphrase):
            seen.append((operation.data_root, operation.argv))
            if len(seen) == 1:
                started.set()
                release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as source, tempfile.TemporaryDirectory() as replacement, tempfile.TemporaryDirectory() as parent, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            alias = Path(parent) / "project"
            alias.symlink_to(source, target_is_directory=True)
            service = OperatorService("generation", runner)
            try:
                service.unlock(str(alias), bytearray(b"passphrase"), duration_seconds=None)
                first = service.submit(str(alias), ["status"])
                self.assertTrue(started.wait(1))
                second = service.submit(str(alias), ["status"])
                canonical_root = str(Path(source).resolve())
                queued = service._operations[second["operation_id"]]
                self.assertEqual(queued.data_root, canonical_root)
                self.assertEqual(queued.argv[:2], ["--data-root", canonical_root])

                alias.unlink()
                alias.symlink_to(replacement, target_is_directory=True)
                release.set()
                self.assertEqual(
                    self._wait_terminal(service, first["operation_id"])["state"],
                    "completed",
                )
                self.assertEqual(
                    self._wait_terminal(service, second["operation_id"])["state"],
                    "completed",
                )
                self.assertTrue(
                    all(data_root == canonical_root for data_root, _argv in seen)
                )
                self.assertTrue(
                    all(argv[:2] == ["--data-root", canonical_root] for _root, argv in seen)
                )
            finally:
                release.set()
                service.close()

    def test_same_project_backup_install_requires_manual_locked_workflow(self) -> None:
        called = False

        def runner(_operation, _passphrase):
            nonlocal called
            called = True
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.submit(
                        tmp,
                        ["backup", "import", "archive.kassiber", "--install"],
                    )
                self.assertEqual(
                    raised.exception.code,
                    "operator_command_not_brokerable",
                )
                self.assertFalse(called)
                self.assertFalse(service._operations)
                self.assertEqual(service.status(tmp)["lease"], "unlocked")
            finally:
                service.close()

    def test_explicit_scope_is_pinned_at_admission(self) -> None:
        connection = mock.Mock()
        started = threading.Event()
        release = threading.Event()
        seen: list[list[str]] = []

        def runner(operation, _passphrase):
            seen.append(operation.argv)
            if len(seen) == 1:
                started.set()
                release.wait(2)
            return OperationResult(0, "", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=connection
        ), mock.patch(
            "kassiber.operator.service.database_instance_id",
            return_value="d" * 32,
        ), mock.patch(
            "kassiber.operator.service.current_context_snapshot",
            return_value={"workspace_id": "workspace-a", "profile_id": "book-a"},
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                first = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))
                queued = service.submit(
                    tmp,
                    [
                        "transactions",
                        "list",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-a",
                    ],
                )
                lease = next(iter(service._leases.values()))
                lease.workspace = "workspace-b"
                lease.profile = "book-b"
                release.set()
                self._wait_terminal(service, first["operation_id"])
                self._wait_terminal(service, queued["operation_id"])
                self.assertEqual(
                    seen[1][:8],
                    [
                        "--data-root",
                        str(Path(tmp).resolve()),
                        "transactions",
                        "list",
                        "--workspace",
                        "workspace-a",
                        "--profile",
                        "book-a",
                    ],
                )
            finally:
                release.set()
                service.close()

    def test_scoped_command_without_explicit_book_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, ["transactions", "list"])
                self.assertEqual(raised.exception.code, "operator_scope_required")
                self.assertEqual(
                    raised.exception.details["missing"],
                    ["workspace", "profile"],
                )
            finally:
                service.close()

    def test_missing_lease_precedes_scope_validation(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                with self.assertRaises(AppError) as raised:
                    service.submit(tmp, ["transactions", "list"])
                self.assertEqual(raised.exception.code, "interaction_required")
            finally:
                service.close()

    def test_context_workspace_change_does_not_inject_the_old_profile(self) -> None:
        seen: list[list[str]] = []

        def runner(operation, _passphrase):
            seen.append(operation.argv)
            return OperationResult(1, "", "validation stop\n")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                lease = next(iter(service._leases.values()))
                lease.workspace = "workspace-a"
                lease.profile = "book-a"
                accepted = service.submit(
                    tmp,
                    ["context", "set", "--workspace", "workspace-b"],
                )
                self._wait_terminal(service, accepted["operation_id"])
                self.assertIn("workspace-b", seen[0])
                self.assertNotIn("--profile", seen[0])
                self.assertNotIn("book-a", seen[0])
            finally:
                service.close()

    def test_blocked_unlock_does_not_block_another_project(self) -> None:
        unlock_entered = threading.Event()
        release_unlock = threading.Event()
        lock_finished = threading.Event()
        errors: list[BaseException] = []

        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
        ):
            first_root = str(Path(first).resolve())

            def opened(data_root, **_kwargs):
                if str(Path(data_root).resolve()) == first_root:
                    unlock_entered.set()
                    if not release_unlock.wait(2):
                        raise AssertionError("timed out releasing blocked unlock")
                return _Connection()

            with mock.patch(
                "kassiber.operator.service.open_db",
                side_effect=opened,
            ), mock.patch(
                "kassiber.operator.service.set_unlock_mode",
                return_value="brokered",
            ):
                service = OperatorService(
                    "generation",
                    lambda *_args: OperationResult(0, "", ""),
                )
                try:
                    service.unlock(
                        second,
                        bytearray(b"second"),
                        duration_seconds=None,
                    )

                    def unlock_first() -> None:
                        try:
                            service.unlock(
                                first,
                                bytearray(b"first"),
                                duration_seconds=None,
                            )
                        except BaseException as exc:
                            errors.append(exc)

                    def lock_first() -> None:
                        try:
                            service.lock(first)
                        except BaseException as exc:
                            errors.append(exc)
                        finally:
                            lock_finished.set()

                    unlock_thread = threading.Thread(target=unlock_first)
                    unlock_thread.start()
                    self.assertTrue(unlock_entered.wait(1))

                    lock_thread = threading.Thread(target=lock_first)
                    lock_thread.start()
                    self.assertFalse(lock_finished.wait(0.05))

                    self.assertEqual(service.status(second)["lease"], "unlocked")
                    accepted = service.submit(second, ["status"])
                    terminal = self._wait_terminal(
                        service,
                        accepted["operation_id"],
                    )
                    self.assertEqual(terminal["state"], "completed")
                finally:
                    release_unlock.set()
                    unlock_thread.join(2)
                    lock_thread.join(2)
                    service.close()
                self.assertFalse(unlock_thread.is_alive())
                self.assertFalse(lock_thread.is_alive())
                self.assertEqual(errors, [])
                self.assertTrue(lock_finished.is_set())

    def test_blocked_fresh_auth_does_not_block_another_project(self) -> None:
        auth_entered = threading.Event()
        release_auth = threading.Event()
        errors: list[BaseException] = []

        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
            mock.patch(
                "kassiber.operator.service.open_db",
                return_value=_Connection(),
            ),
            mock.patch(
                "kassiber.operator.service.set_unlock_mode",
                return_value="brokered",
            ),
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(first, bytearray(b"first"), duration_seconds=None)
                service.unlock(second, bytearray(b"second"), duration_seconds=None)
                first_root = str(Path(first).resolve())

                def opened(data_root, **_kwargs):
                    if str(Path(data_root).resolve()) == first_root:
                        auth_entered.set()
                        if not release_auth.wait(2):
                            raise AssertionError("timed out releasing blocked auth")
                    return _Connection()

                def authenticate_first() -> None:
                    try:
                        service.authenticate_database(
                            first,
                            bytearray(b"first"),
                            scope="operator_admin",
                        )
                    except BaseException as exc:
                        errors.append(exc)

                with mock.patch(
                    "kassiber.operator.service.open_db",
                    side_effect=opened,
                ):
                    auth_thread = threading.Thread(target=authenticate_first)
                    auth_thread.start()
                    self.assertTrue(auth_entered.wait(1))
                    self.assertEqual(service.status(second)["lease"], "unlocked")
                    accepted = service.submit(second, ["status"])
                    terminal = self._wait_terminal(
                        service,
                        accepted["operation_id"],
                    )
                    self.assertEqual(terminal["state"], "completed")
                    release_auth.set()
                    auth_thread.join(2)
                self.assertFalse(auth_thread.is_alive())
                self.assertEqual(errors, [])
            finally:
                release_auth.set()
                service.close()

    def test_blocked_authenticated_continuation_does_not_block_other_project(
        self,
    ) -> None:
        continuation_entered = threading.Event()
        release_continuation = threading.Event()
        errors: list[BaseException] = []

        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
            mock.patch(
                "kassiber.operator.service.open_db",
                return_value=_Connection(),
            ),
            mock.patch(
                "kassiber.operator.service.set_unlock_mode",
                return_value="brokered",
            ),
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(first, bytearray(b"first"), duration_seconds=None)
                service.unlock(second, bytearray(b"second"), duration_seconds=None)

                def continuation(_database_identity: str) -> None:
                    continuation_entered.set()
                    if not release_continuation.wait(2):
                        raise AssertionError("timed out releasing continuation")

                def authenticate_first() -> None:
                    try:
                        service.authenticate_database(
                            first,
                            bytearray(b"first"),
                            scope="operator_native_enrollment",
                            continuation=continuation,
                        )
                    except BaseException as exc:
                        errors.append(exc)

                auth_thread = threading.Thread(target=authenticate_first)
                auth_thread.start()
                self.assertTrue(continuation_entered.wait(1))
                self.assertEqual(service.status(second)["lease"], "unlocked")
                accepted = service.submit(second, ["status"])
                self.assertEqual(
                    self._wait_terminal(service, accepted["operation_id"])["state"],
                    "completed",
                )
                release_continuation.set()
                auth_thread.join(2)
                self.assertFalse(auth_thread.is_alive())
                self.assertEqual(errors, [])
            finally:
                release_continuation.set()
                service.close()

    def test_close_waits_for_an_authenticated_continuation(self) -> None:
        continuation_entered = threading.Event()
        release_continuation = threading.Event()
        close_finished = threading.Event()
        errors: list[BaseException] = []

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ), mock.patch(
            "kassiber.operator.service.set_unlock_mode",
            return_value="brokered",
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)

            def continuation(_database_identity: str) -> None:
                continuation_entered.set()
                if not release_continuation.wait(2):
                    raise AssertionError("timed out releasing continuation")

            def authenticate() -> None:
                try:
                    service.authenticate_database(
                        tmp,
                        bytearray(b"passphrase"),
                        scope="operator_native_enrollment",
                        continuation=continuation,
                    )
                except BaseException as exc:
                    errors.append(exc)

            def close_service() -> None:
                try:
                    service.close()
                except BaseException as exc:
                    errors.append(exc)
                finally:
                    close_finished.set()

            auth_thread = threading.Thread(target=authenticate)
            close_thread = threading.Thread(target=close_service)
            try:
                auth_thread.start()
                self.assertTrue(continuation_entered.wait(1))
                close_thread.start()
                self.assertFalse(close_finished.wait(0.05))
                release_continuation.set()
                auth_thread.join(2)
                close_thread.join(2)
                self.assertFalse(auth_thread.is_alive())
                self.assertFalse(close_thread.is_alive())
                self.assertTrue(close_finished.is_set())
                self.assertEqual(errors, [])
                self.assertFalse(service._leases)
                self.assertFalse(service._workers)
            finally:
                release_continuation.set()
                service.close()

    def test_close_during_unlock_prevents_lease_installation(self) -> None:
        unlock_entered = threading.Event()
        release_unlock = threading.Event()
        close_finished = threading.Event()
        errors: list[BaseException] = []

        class BlockingConnection(_Connection):
            pass

        def opened(*_args, **_kwargs):
            unlock_entered.set()
            if not release_unlock.wait(2):
                raise AssertionError("timed out releasing unlock")
            return BlockingConnection()

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", side_effect=opened
        ), mock.patch(
            "kassiber.operator.service.set_unlock_mode",
            return_value="brokered",
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )

            def unlock() -> None:
                try:
                    service.unlock(
                        tmp,
                        bytearray(b"passphrase"),
                        duration_seconds=None,
                    )
                except BaseException as exc:
                    errors.append(exc)

            def close_service() -> None:
                service.close()
                close_finished.set()

            unlock_thread = threading.Thread(target=unlock)
            close_thread = threading.Thread(target=close_service)
            try:
                unlock_thread.start()
                self.assertTrue(unlock_entered.wait(1))
                close_thread.start()
                self.assertFalse(close_finished.wait(0.05))
                release_unlock.set()
                unlock_thread.join(2)
                close_thread.join(2)
                self.assertFalse(unlock_thread.is_alive())
                self.assertFalse(close_thread.is_alive())
                self.assertTrue(close_finished.is_set())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], AppError)
                self.assertEqual(errors[0].code, "operator_broker_stopped")
                self.assertFalse(service._leases)
                self.assertFalse(service._workers)
            finally:
                release_unlock.set()
                service.close()

    def test_close_allows_a_running_atomic_operation_to_finish(self) -> None:
        started = threading.Event()
        release = threading.Event()

        def runner(_operation, _passphrase):
            started.set()
            if not release.wait(2):
                raise AssertionError("timed out releasing running operation")
            return OperationResult(0, "finished\n", "")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ):
            service = OperatorService("generation", runner)
            try:
                service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                retained = next(iter(service._leases.values())).passphrase
                accepted = service.submit(tmp, ["status"])
                self.assertTrue(started.wait(1))

                service.close()
                self.assertEqual(
                    service.operation_status(accepted["operation_id"])["state"],
                    "running",
                )
                release.set()
                terminal = self._wait_terminal(service, accepted["operation_id"])
                self.assertEqual(terminal["state"], "completed")
                self.assertEqual(terminal["stdout"], "finished\n")
                self.assertEqual(set(retained), {0})
            finally:
                release.set()
                service.close()

    def test_close_retries_failed_owner_release_without_repreparing(self) -> None:
        owner = mock.Mock()
        owner.release.side_effect = [OSError("transient release failure"), None]

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
            with mock.patch.object(
                service,
                "_revoke_lease_locked",
                wraps=service._revoke_lease_locked,
            ) as revoke:
                with self.assertRaisesRegex(OSError, "transient release failure"):
                    service.close()
                self.assertEqual(owner.release.call_count, 1)
                self.assertEqual(revoke.call_count, 1)

                service.close()

                self.assertEqual(owner.release.call_count, 2)
                self.assertEqual(revoke.call_count, 1)
                self.assertFalse(service._pending_owner_releases)

    def test_close_interrupt_restores_state_and_retries_safe_cleanup(self) -> None:
        service = OperatorService(
            "generation",
            lambda *_args: OperationResult(0, "", ""),
        )
        with mock.patch.object(
            service,
            "_release_pending_owners",
            side_effect=[KeyboardInterrupt, None],
        ) as release:
            with self.assertRaises(KeyboardInterrupt):
                service.close()

            self.assertFalse(service._close_in_progress)
            self.assertFalse(service._close_complete.is_set())

            service.close()

        self.assertEqual(release.call_count, 2)
        self.assertTrue(service._close_complete.is_set())

    def test_concurrent_close_waiter_retries_failed_owner_release(self) -> None:
        first_release_entered = threading.Event()
        allow_first_failure = threading.Event()
        second_close_finished = threading.Event()
        release_attempts = 0
        release_guard = threading.Lock()
        errors: list[BaseException] = []

        def release_owner() -> None:
            nonlocal release_attempts
            with release_guard:
                release_attempts += 1
                attempt = release_attempts
            if attempt == 1:
                first_release_entered.set()
                if not allow_first_failure.wait(2):
                    raise AssertionError("timed out releasing first close")
                raise OSError("transient release failure")

        owner = mock.Mock()
        owner.release.side_effect = release_owner

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db",
            return_value=_Connection(),
        ), mock.patch(
            "kassiber.operator.service.acquire_project_ownership",
            return_value=owner,
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)

            def first_close() -> None:
                try:
                    service.close()
                except BaseException as exc:
                    errors.append(exc)

            def second_close() -> None:
                try:
                    service.close()
                except BaseException as exc:
                    errors.append(exc)
                finally:
                    second_close_finished.set()

            first_thread = threading.Thread(target=first_close)
            second_thread = threading.Thread(target=second_close)
            try:
                first_thread.start()
                self.assertTrue(first_release_entered.wait(1))
                second_thread.start()
                self.assertFalse(second_close_finished.wait(0.05))

                allow_first_failure.set()
                first_thread.join(2)
                second_thread.join(2)

                self.assertFalse(first_thread.is_alive())
                self.assertFalse(second_thread.is_alive())
                self.assertTrue(second_close_finished.is_set())
                self.assertEqual(len(errors), 1)
                self.assertIsInstance(errors[0], OSError)
                self.assertEqual(release_attempts, 2)
                self.assertFalse(service._pending_owner_releases)
            finally:
                allow_first_failure.set()
                service.close()

    def test_blocked_scope_refresh_does_not_block_another_project(self) -> None:
        refresh_entered = threading.Event()
        release_refresh = threading.Event()

        with (
            tempfile.TemporaryDirectory() as first,
            tempfile.TemporaryDirectory() as second,
            mock.patch(
                "kassiber.operator.service.open_db",
                return_value=_Connection(),
            ),
            mock.patch(
                "kassiber.operator.service.current_context_snapshot",
                return_value={},
            ),
            mock.patch(
                "kassiber.operator.service.set_unlock_mode",
                return_value="brokered",
            ),
        ):
            service = OperatorService(
                "generation",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                service.unlock(first, bytearray(b"first"), duration_seconds=None)
                service.unlock(second, bytearray(b"second"), duration_seconds=None)
                first_root = str(Path(first).resolve())

                def opened(data_root, **_kwargs):
                    if str(Path(data_root).resolve()) == first_root:
                        refresh_entered.set()
                        if not release_refresh.wait(2):
                            raise AssertionError("timed out releasing scope refresh")
                    return _Connection()

                with mock.patch(
                    "kassiber.operator.service.open_db",
                    side_effect=opened,
                ):
                    service.submit(
                        first,
                        ["context", "set", "--workspace", "workspace-a"],
                    )
                    self.assertTrue(refresh_entered.wait(1))
                    self.assertEqual(service.status(second)["lease"], "unlocked")
                    accepted = service.submit(second, ["status"])
                    terminal = self._wait_terminal(
                        service,
                        accepted["operation_id"],
                    )
                    self.assertEqual(terminal["state"], "completed")
                    release_refresh.set()
                    service.status(first)
            finally:
                release_refresh.set()
                service.close()

    def test_authentication_attempts_are_serialized_per_project(self) -> None:
        barrier = threading.Barrier(3)
        guard = threading.Lock()
        active = 0
        max_active = 0

        def opened(
            _data_root,
            *,
            passphrase,
            require_existing_schema,
            expected_database_identity=None,
        ):
            nonlocal active, max_active
            if passphrase == "correct":
                return _Connection()
            with guard:
                active += 1
                max_active = max(max_active, active)
            time.sleep(0.05)
            with guard:
                active -= 1
            raise AppError("wrong", code="unlock_failed")

        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", side_effect=opened
        ):
            service = OperatorService("generation", lambda *_args: OperationResult(0, "", ""))
            service.unlock(tmp, bytearray(b"correct"), duration_seconds=None)
            errors: list[str] = []

            def attempt() -> None:
                barrier.wait()
                try:
                    service.authenticate_database(tmp, bytearray(b"wrong"), scope="test")
                except AppError as exc:
                    errors.append(exc.code)

            threads = [threading.Thread(target=attempt) for _ in range(2)]
            for thread in threads:
                thread.start()
            barrier.wait()
            for thread in threads:
                thread.join(2)
            try:
                self.assertEqual(errors, ["unlock_failed", "unlock_failed"])
                self.assertEqual(max_active, 1)
            finally:
                service.close()

    @staticmethod
    def _wait(service: OperatorService, operation_id: str) -> dict[str, object]:
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            status = service.operation_status(operation_id)
            if status["state"] in {"completed", "failed"}:
                return status
            time.sleep(0.01)
        raise AssertionError("operation did not finish")

    @staticmethod
    def _wait_terminal(service: OperatorService, operation_id: str) -> dict[str, object]:
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            status = service.operation_status(operation_id)
            if status["state"] in {"completed", "failed", "cancelled", "result_unknown"}:
                return status
            time.sleep(0.01)
        raise AssertionError("operation did not finish")


class OperatorClientArgumentTest(unittest.TestCase):
    def test_dead_broker_operation_status_is_result_unknown(self) -> None:
        client = BrokerClient()
        with mock.patch.object(client, "_simple_request", side_effect=ConnectionRefusedError):
            status = client.operation_status("generation.client.operation")
        self.assertEqual(status["state"], "result_unknown")
        self.assertEqual(status["reason"], "broker_unreachable")

    def test_frozen_sidecar_launch_commands_do_not_use_python_dash_m(self) -> None:
        with mock.patch("kassiber.operator.launcher.sys.frozen", True, create=True), mock.patch(
            "kassiber.operator.launcher.sys.executable", "/bundle/kassiber-cli"
        ):
            self.assertEqual(cli_child_command(), ["/bundle/kassiber-cli"])
            self.assertEqual(
                broker_server_command(),
                ["/bundle/kassiber-cli", "--operator-broker-server"],
            )

    def test_frozen_sidecar_children_reset_pyinstaller_runtime(self) -> None:
        environment = {"EXISTING": "value"}
        with mock.patch("kassiber.operator.launcher.sys.frozen", True, create=True):
            prepare_independent_child_environment(environment)
        self.assertEqual(environment["EXISTING"], "value")
        self.assertEqual(environment["PYINSTALLER_RESET_ENVIRONMENT"], "1")

    def test_source_children_do_not_gain_pyinstaller_runtime_setting(self) -> None:
        environment: dict[str, str] = {}
        with mock.patch("kassiber.operator.launcher.sys.frozen", False, create=True):
            prepare_independent_child_environment(environment)
        self.assertNotIn("PYINSTALLER_RESET_ENVIRONMENT", environment)

    def test_frozen_broker_allows_for_one_file_extraction(self) -> None:
        with mock.patch(
            "kassiber.operator.client.sys.frozen", True, create=True
        ), mock.patch("kassiber.operator.client.os.name", "posix"):
            self.assertEqual(
                _broker_startup_timeout_seconds(),
                FROZEN_BROKER_STARTUP_TIMEOUT_SECONDS,
            )
        with mock.patch(
            "kassiber.operator.client.sys.frozen", True, create=True
        ), mock.patch("kassiber.operator.client.os.name", "nt"):
            self.assertEqual(
                _broker_startup_timeout_seconds(),
                WINDOWS_FROZEN_BROKER_STARTUP_TIMEOUT_SECONDS,
            )
        with mock.patch("kassiber.operator.client.sys.frozen", False, create=True):
            self.assertEqual(
                _broker_startup_timeout_seconds(),
                SOURCE_BROKER_STARTUP_TIMEOUT_SECONDS,
            )

    def test_secret_fd_is_replaced_by_opaque_label(self) -> None:
        read_fd, write_fd = os.pipe()
        os.write(write_fd, b"secret-value\n")
        os.close(write_fd)
        prepared = prepare_arguments(["backends", "create", "--token-fd", str(read_fd)])
        try:
            self.assertEqual(prepared.argv[:3], ["backends", "create", "--token-fd"])
            label = prepared.argv[3]
            self.assertTrue(label.startswith("broker-secret-"))
            self.assertEqual(bytes(prepared.secrets[label]), b"secret-value")
            self.assertNotIn("secret-value", repr(prepared.argv))
        finally:
            wipe_prepared(prepared)

    def test_inline_secret_fd_is_replaced_by_opaque_label(self) -> None:
        read_fd, write_fd = os.pipe()
        os.write(write_fd, b"inline-secret\n")
        os.close(write_fd)
        prepared = prepare_arguments(
            ["backends", "create", f"--token-fd={read_fd}"]
        )
        try:
            self.assertEqual(prepared.argv[:3], ["backends", "create", "--token-fd"])
            label = prepared.argv[3]
            self.assertTrue(label.startswith("broker-secret-"))
            self.assertEqual(bytes(prepared.secrets[label]), b"inline-secret")
            self.assertNotIn(str(read_fd), prepared.argv)
        finally:
            wipe_prepared(prepared)

    def test_inline_secret_fd_rejects_malformed_descriptor(self) -> None:
        with self.assertRaises(AppError) as raised:
            prepare_arguments(["backends", "create", "--token-fd=not-an-fd"])

        self.assertEqual(raised.exception.code, "operator_invalid_command")

    def test_duration_parser_has_no_arbitrary_session_cap(self) -> None:
        self.assertEqual(parse_duration("8h"), 28_800)
        self.assertEqual(parse_duration("30d"), 2_592_000)
        self.assertEqual(parse_duration("5000d"), 432_000_000)
        for invalid in ("59s", "0h", "8", "8w"):
            with self.subTest(invalid=invalid), self.assertRaises(AppError):
                parse_duration(invalid)


if __name__ == "__main__":
    unittest.main()
