from __future__ import annotations

import multiprocessing
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from kassiber.operator import project as project_module
from kassiber.errors import AppError
from kassiber.operator.project import (
    CanonicalProject,
    ProjectOwnerChildHandles,
    ProjectOwnerLease,
    acquire_project_ownership,
    canonical_project,
    exclusive_project_maintenance,
)
from kassiber.operator.protocol import operator_runtime_dir
from kassiber.operator.service import OperationResult, OperatorService
from kassiber import daemon as daemon_runtime


class _Connection:
    def close(self) -> None:
        pass


def _competing_owner(
    data_root: str,
    owner_kind: str,
    output: multiprocessing.Queue,
) -> None:
    project = canonical_project(data_root)
    try:
        lease = acquire_project_ownership(
            project,
            owner_kind=owner_kind,
            generation=f"{owner_kind}-test",
        )
    except AppError as exc:
        output.put((exc.code, exc.details))
    else:
        lease.release()
        output.put(("acquired", {}))


def _reported_locked_owner(owner: str) -> str:
    # POSIX flock permits reading the already-open lock record after the lock
    # attempt fails. The Windows writer share policy prevents that competing
    # writer open, so the contender proves exclusion without the owner label.
    return "unknown" if os.name == "nt" else owner


def _try_legacy_exclusive_lock(
    lock_path: Path,
    project_id: str,
):
    if os.name != "nt":
        handle = project_module._open_owner_lock(lock_path, project_id)
        if not project_module._try_lock_handle(handle):
            handle.close()
            return None
        return handle

    import ctypes
    from ctypes import wintypes

    create_file = ctypes.WinDLL("kernel32", use_last_error=True).CreateFileW
    create_file.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    ]
    create_file.restype = wintypes.HANDLE
    handle = create_file(
        str(lock_path),
        0x80000000 | 0x40000000,  # GENERIC_READ | GENERIC_WRITE
        0,  # legacy versions denied all sharing
        None,
        4,  # OPEN_ALWAYS
        0x80,  # FILE_ATTRIBUTE_NORMAL
        None,
    )
    invalid_handle = wintypes.HANDLE(-1).value
    if handle == invalid_handle:
        error = ctypes.get_last_error()
        if error in {32, 33}:  # ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
            return None
        raise OSError(error, f"CreateFileW failed for {lock_path}")
    return handle


def _close_legacy_exclusive_lock(handle) -> None:
    if os.name == "nt":
        import ctypes

        if not ctypes.WinDLL("kernel32", use_last_error=True).CloseHandle(handle):
            raise ctypes.WinError(ctypes.get_last_error())
        return
    project_module._unlock_handle(handle)
    handle.close()


class OperatorProjectTest(unittest.TestCase):
    def test_parent_owner_release_attempts_every_handle_and_can_retry(self) -> None:
        first = mock.Mock()
        second = mock.Mock()
        second.close.side_effect = [OSError("second close failed"), None]
        owner = ProjectOwnerLease(
            project=mock.Mock(),
            owner_kind="broker",
            generation="generation",
            _handles=(first, second),
            _lock_paths=set(),
        )

        with self.assertRaisesRegex(OSError, "second close failed"):
            owner.release()
        first.close.assert_called_once_with()
        second.close.assert_called_once_with()

        owner.release()
        self.assertEqual(first.close.call_count, 2)
        self.assertEqual(second.close.call_count, 2)
        owner.release()
        self.assertEqual(first.close.call_count, 2)
        self.assertEqual(second.close.call_count, 2)

    def test_child_owner_close_attempts_every_handle(self) -> None:
        first = mock.Mock()
        first.close.side_effect = OSError("first close failed")
        second = mock.Mock()
        handles = ProjectOwnerChildHandles((1, 2), (first, second))

        with self.assertRaisesRegex(OSError, "first close failed"):
            handles.close()

        first.close.assert_called_once_with()
        second.close.assert_called_once_with()

    def test_symlink_aliases_share_existing_database_identity(self) -> None:
        if os.name == "nt":
            self.skipTest("symlink privileges vary on Windows")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "real"
            root.mkdir()
            (root / "kassiber.sqlite3").write_bytes(b"database")
            alias = Path(tmp) / "alias"
            alias.symlink_to(root, target_is_directory=True)
            self.assertEqual(
                canonical_project(root).identity,
                canonical_project(alias).identity,
            )

    def test_hardlink_aliases_are_rejected_before_project_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            first = Path(tmp) / "first"
            second = Path(tmp) / "second"
            first.mkdir()
            second.mkdir()
            database = first / "kassiber.sqlite3"
            database.write_bytes(b"database")
            os.link(database, second / "kassiber.sqlite3")

            for data_root in (first, second):
                with self.subTest(data_root=data_root):
                    with self.assertRaises(AppError) as raised:
                        canonical_project(data_root)
                    self.assertEqual(raised.exception.code, "unsafe_project_database")
                    self.assertEqual(raised.exception.details, {"link_count": 2})

    def test_competing_process_gets_public_safe_owner_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            lease = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker-generation",
            )
            try:
                context = multiprocessing.get_context("spawn")
                output: multiprocessing.Queue = context.Queue()
                process = context.Process(
                    target=_competing_owner,
                    args=(tmp, "broker", output),
                )
                process.start()
                process.join(5)
                self.assertEqual(process.exitcode, 0)
                code, details = output.get(timeout=1)
                self.assertEqual(code, "project_in_use")
                self.assertEqual(details["project"], project.public_id)
                self.assertEqual(
                    details["owner"],
                    _reported_locked_owner("broker"),
                )
                self.assertNotIn(str(Path(tmp)), repr(details))
            finally:
                lease.release()

    def test_desktop_and_broker_can_coexist_across_processes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            desktop = acquire_project_ownership(
                canonical_project(tmp),
                owner_kind="desktop",
                generation="desktop",
            )
            try:
                context = multiprocessing.get_context("spawn")
                output: multiprocessing.Queue = context.Queue()
                process = context.Process(
                    target=_competing_owner,
                    args=(tmp, "broker", output),
                )
                process.start()
                process.join(5)
                self.assertEqual(process.exitcode, 0)
                self.assertEqual(output.get(timeout=1), ("acquired", {}))
            finally:
                desktop.release()

    def test_simultaneous_desktop_and_broker_admission_both_succeed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            desktop_in_probe = threading.Event()
            continue_desktop = threading.Event()
            original_probe = project_module._active_owner_record
            leases: list[ProjectOwnerLease] = []
            errors: list[BaseException] = []

            def paused_probe(lock_path: Path, project_id: str):
                if (
                    lock_path.name.endswith(".broker")
                    and not desktop_in_probe.is_set()
                ):
                    desktop_in_probe.set()
                    self.assertTrue(continue_desktop.wait(2))
                return original_probe(lock_path, project_id)

            def acquire(owner_kind: str) -> None:
                try:
                    leases.append(
                        acquire_project_ownership(
                            project,
                            owner_kind=owner_kind,
                            generation=owner_kind,
                        )
                    )
                except BaseException as exc:
                    errors.append(exc)

            with mock.patch.object(
                project_module,
                "_active_owner_record",
                side_effect=paused_probe,
            ):
                desktop_thread = threading.Thread(target=acquire, args=("desktop",))
                broker_thread = threading.Thread(target=acquire, args=("broker",))
                desktop_thread.start()
                self.assertTrue(desktop_in_probe.wait(2))
                broker_thread.start()
                continue_desktop.set()
                desktop_thread.join(5)
                broker_thread.join(5)

            try:
                self.assertFalse(desktop_thread.is_alive())
                self.assertFalse(broker_thread.is_alive())
                self.assertEqual(errors, [])
                self.assertCountEqual(
                    [lease.owner_kind for lease in leases],
                    ["desktop", "broker"],
                )
            finally:
                for lease in reversed(leases):
                    lease.release()

    def test_database_maintenance_excludes_the_other_live_role(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            desktop = acquire_project_ownership(
                project,
                owner_kind="desktop",
                generation="desktop",
            )
            try:
                with self.assertRaises(AppError) as raised:
                    with exclusive_project_maintenance(
                        tmp,
                        active_owner_kind="broker",
                    ):
                        self.fail("maintenance must not overlap the desktop")
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                desktop.release()

            broker = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker",
            )
            try:
                with self.assertRaises(AppError) as raised:
                    with exclusive_project_maintenance(
                        tmp,
                        active_owner_kind="desktop",
                    ):
                        self.fail("maintenance must not overlap the broker")
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                broker.release()

            with exclusive_project_maintenance(tmp, active_owner_kind=None):
                pass

    def test_rejected_alias_rolls_back_only_its_new_locks(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            first_root = Path(tmp) / "first"
            alias_root = Path(tmp) / "alias"
            first_root.mkdir()
            alias_root.mkdir()
            first_database = first_root / "kassiber.sqlite3"
            first_database.write_bytes(b"database")
            project = canonical_project(first_root)
            owner = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker",
            )
            original_handles = owner._handles
            original_lock_paths = owner._lock_paths.copy()
            alias = CanonicalProject(
                database=alias_root / "kassiber.sqlite3",
                lock_path=project.lock_path,
                alias_lock_path=project_module._owner_lock_root()
                / "path-alias.lock",
                local_lock_path=alias_root / project_module.OWNER_LOCK_FILENAME,
                identity=project.identity,
                public_id=project.public_id,
            )
            try:
                with mock.patch.object(
                    project_module,
                    "_require_compatible_other_owner",
                    side_effect=AppError(
                        "replaced project",
                        code="project_in_use",
                    ),
                ):
                    with self.assertRaises(AppError):
                        owner.add_alias(alias)
                self.assertEqual(owner._handles, original_handles)
                self.assertEqual(owner._lock_paths, original_lock_paths)
            finally:
                owner.release()

    def test_release_allows_next_owner_and_lock_is_private(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            first = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="one",
            )
            first.release()
            second = acquire_project_ownership(
                project,
                owner_kind="desktop",
                generation="two",
            )
            second.release()
            if os.name != "nt":
                for lock_path in (
                    project.lock_path,
                    project.alias_lock_path,
                    project.local_lock_path,
                ):
                    self.assertEqual(lock_path.stat().st_mode & 0o777, 0o600)

    def test_owner_exclusion_does_not_follow_broker_runtime_override(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            with mock.patch.dict(os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": first}):
                lease = acquire_project_ownership(
                    canonical_project(tmp),
                    owner_kind="broker",
                    generation="broker",
                )
            try:
                with mock.patch.dict(os.environ, {"KASSIBER_OPERATOR_RUNTIME_DIR": second}):
                    with self.assertRaises(AppError) as raised:
                        acquire_project_ownership(
                            canonical_project(tmp),
                            owner_kind="broker",
                            generation="other-broker",
                        )
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                lease.release()

    @unittest.skipIf(os.name == "nt", "POSIX account-home lock namespace")
    def test_owner_namespace_uses_account_home_not_runtime_environment(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            tempfile.TemporaryDirectory() as account,
        ):
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            with mock.patch("pwd.getpwuid") as getpwuid, mock.patch.dict(
                os.environ,
                {
                    "HOME": str(Path(tmp) / "caller-home"),
                    "XDG_RUNTIME_DIR": str(Path(tmp) / "caller-runtime"),
                    "KASSIBER_OPERATOR_RUNTIME_DIR": str(Path(tmp) / "override"),
                },
            ):
                getpwuid.return_value = SimpleNamespace(pw_dir=account)
                project = canonical_project(tmp)
            self.assertEqual(
                project.lock_path.parent,
                Path(account).resolve()
                / ".kassiber"
                / "run"
                / "operator-owners",
            )

    @unittest.skipIf(os.name == "nt", "POSIX account-home runtime permissions")
    def test_owner_namespace_keeps_shared_broker_runtime_private(self) -> None:
        with (
            tempfile.TemporaryDirectory() as tmp,
            tempfile.TemporaryDirectory() as account,
            mock.patch(
                "pwd.getpwuid",
                return_value=SimpleNamespace(pw_dir=account),
            ),
        ):
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            canonical_project(tmp)
            runtime = Path(account) / ".kassiber" / "run"
            self.assertEqual(runtime.stat().st_mode & 0o777, 0o700)
            self.assertEqual(operator_runtime_dir(), runtime.resolve())

    def test_broker_first_allows_desktop_owner(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            project = canonical_project(tmp)
            broker = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker",
            )
            ctx = SimpleNamespace(
                data_root=tmp,
                project_owner=None,
                ownership_generation="desktop",
            )
            try:
                desktop = daemon_runtime._ensure_daemon_project_owner(ctx)
                self.assertEqual(desktop.owner_kind, "desktop")
                self.assertEqual(desktop.project.identity, broker.project.identity)
            finally:
                daemon_runtime._release_daemon_project_owner(ctx)
                broker.release()

    def test_desktop_first_allows_broker_unlock(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ) as open_database, mock.patch(
            "kassiber.operator.service.set_unlock_mode",
            return_value="brokered",
        ):
            project = canonical_project(tmp)
            desktop = acquire_project_ownership(
                project,
                owner_kind="desktop",
                generation="desktop",
            )
            service = OperatorService(
                "broker",
                lambda *_args: OperationResult(0, "", ""),
            )
            try:
                status = service.unlock(
                    tmp,
                    bytearray(b"passphrase"),
                    duration_seconds=None,
                )
                self.assertEqual(status["lease"], "unlocked")
                open_database.assert_called_once()
            finally:
                service.close()
                desktop.release()

    def test_desktop_and_broker_can_own_the_same_project(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            project = canonical_project(tmp)
            desktop = acquire_project_ownership(
                project,
                owner_kind="desktop",
                generation="desktop",
            )
            try:
                broker = acquire_project_ownership(
                    project,
                    owner_kind="broker",
                    generation="broker",
                )
            finally:
                desktop.release()
            broker.release()

    def test_broker_owner_allows_daemon_database_open(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.daemon.open_db", return_value=_Connection()
        ) as open_database, mock.patch(
            "kassiber.daemon.validate_project_migration_after_unlock"
        ), mock.patch(
            "kassiber.daemon.merge_db_backends"
        ), mock.patch(
            "kassiber.daemon._remember_unlocked_passphrase"
        ), mock.patch(
            "kassiber.daemon._start_freshness_background_worker"
        ):
            broker = acquire_project_ownership(
                canonical_project(tmp),
                owner_kind="broker",
                generation="broker",
            )
            ctx = SimpleNamespace(
                conn=None,
                data_root=tmp,
                project_owner=None,
                ownership_generation="desktop",
                retired_project_resources=[],
                runtime_config={},
                project_id=None,
                select_project_on_open=False,
            )
            try:
                opened = daemon_runtime._open_daemon_connection(ctx)
                self.assertIs(opened, ctx.conn)
                open_database.assert_called_once()
            finally:
                daemon_runtime._release_daemon_project_owner(ctx)
                broker.release()

    @unittest.skipIf(os.name == "nt", "POSIX symlink test")
    def test_daemon_open_uses_the_owned_projects_canonical_root(self) -> None:
        with tempfile.TemporaryDirectory() as root, tempfile.TemporaryDirectory() as parent:
            Path(root, "kassiber.sqlite3").write_bytes(b"database")
            alias = Path(parent) / "project"
            alias.symlink_to(root, target_is_directory=True)
            connection = _Connection()
            ctx = SimpleNamespace(
                conn=None,
                data_root=str(alias),
                project_owner=None,
                ownership_generation="desktop",
                runtime_config={},
                project_id=None,
                select_project_on_open=False,
                retired_project_resources=[],
            )
            with mock.patch(
                "kassiber.daemon.open_db",
                return_value=connection,
            ) as open_database, mock.patch(
                "kassiber.daemon.validate_project_migration_after_unlock"
            ), mock.patch(
                "kassiber.daemon.merge_db_backends"
            ), mock.patch(
                "kassiber.daemon._remember_unlocked_passphrase"
            ), mock.patch(
                "kassiber.daemon._start_freshness_background_worker"
            ):
                try:
                    opened = daemon_runtime._open_daemon_connection(ctx)
                    self.assertIs(opened, connection)
                    canonical_root = str(Path(root).resolve())
                    self.assertEqual(
                        open_database.call_args.args[0],
                        canonical_root,
                    )
                    self.assertEqual(ctx.data_root, canonical_root)
                finally:
                    daemon_runtime._release_daemon_project_owner(ctx)

    def test_inherited_owner_handles_exclude_a_second_owner_after_parent_release(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            lease = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker",
            )
            inherited = lease.duplicate_for_child()
            popen_args: dict[str, object]
            if os.name == "nt":
                startup = subprocess.STARTUPINFO()
                startup.lpAttributeList = {"handle_list": list(inherited.tokens)}
                popen_args = {"startupinfo": startup, "close_fds": True}
            else:
                popen_args = {"pass_fds": inherited.tokens}
            child = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(0.4)"],
                **popen_args,
            )
            inherited.close()
            lease.release()
            try:
                with self.assertRaises(AppError) as raised:
                    acquire_project_ownership(
                        project,
                        owner_kind="broker",
                        generation="replacement-broker",
                    )
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                child.wait(timeout=2)
            next_owner = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="replacement-broker",
            )
            next_owner.release()

    def test_move_preserves_identity_and_owner_exclusion(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            first = Path(tmp) / "first"
            second = Path(tmp) / "second"
            first.mkdir()
            second.mkdir()
            database = first / "kassiber.sqlite3"
            database.write_bytes(b"database")
            project = canonical_project(first)
            lease = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker",
            )
            database.rename(second / "kassiber.sqlite3")
            moved = canonical_project(second)
            try:
                self.assertEqual(project.identity, moved.identity)
                with self.assertRaises(AppError) as raised:
                    acquire_project_ownership(
                        moved,
                        owner_kind="broker",
                        generation="replacement-broker",
                    )
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                lease.release()

    def test_path_lock_blocks_replacement_inode(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            database = Path(tmp) / "kassiber.sqlite3"
            database.write_bytes(b"first")
            lease = acquire_project_ownership(
                canonical_project(tmp),
                owner_kind="broker",
                generation="broker",
            )
            database.rename(Path(tmp) / "old.sqlite3")
            database.write_bytes(b"replacement")
            try:
                with self.assertRaises(AppError) as raised:
                    acquire_project_ownership(
                        canonical_project(tmp),
                        owner_kind="desktop",
                        generation="desktop",
                    )
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                lease.release()

    def test_legacy_exclusive_owner_blocks_new_role_owner(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            legacy = _try_legacy_exclusive_lock(
                project.lock_path,
                project.public_id,
            )
            self.assertIsNotNone(legacy)
            try:
                with mock.patch.object(
                    project_module,
                    "_read_owner_record",
                    return_value={
                        "owner": "desktop",
                        "generation": "legacy-desktop",
                    },
                ):
                    with self.assertRaises(AppError) as raised:
                        acquire_project_ownership(
                            project,
                            owner_kind="desktop",
                            generation="desktop",
                        )
                self.assertEqual(raised.exception.code, "project_in_use")
                if os.name != "nt":
                    self.assertIn("desktop app or preview", str(raised.exception))
                    self.assertIn("second desktop", raised.exception.hint)
            finally:
                _close_legacy_exclusive_lock(legacy)

    def test_new_role_owner_blocks_legacy_exclusive_owner(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, "kassiber.sqlite3").write_bytes(b"database")
            project = canonical_project(tmp)
            owner = acquire_project_ownership(
                project,
                owner_kind="broker",
                generation="broker",
            )
            try:
                legacy = _try_legacy_exclusive_lock(
                    project.lock_path,
                    project.public_id,
                )
                self.assertIsNone(legacy)
            finally:
                owner.release()


if __name__ == "__main__":
    unittest.main()
