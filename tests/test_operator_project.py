from __future__ import annotations

import multiprocessing
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from kassiber.errors import AppError
from kassiber.operator.project import (
    ProjectOwnerChildHandles,
    ProjectOwnerLease,
    acquire_project_ownership,
    canonical_project,
)
from kassiber.operator.service import OperationResult, OperatorService
from kassiber import daemon as daemon_runtime


class _Connection:
    def close(self) -> None:
        pass


def _competing_owner(data_root: str, output: multiprocessing.Queue) -> None:
    project = canonical_project(data_root)
    try:
        lease = acquire_project_ownership(
            project,
            owner_kind="desktop",
            generation="desktop-test",
        )
    except AppError as exc:
        output.put((exc.code, exc.details))
    else:
        lease.release()
        output.put(("acquired", {}))


def _reported_locked_owner(owner: str) -> str:
    # POSIX flock permits reading the already-open lock record after the lock
    # attempt fails. Windows share-mode zero correctly prevents that second
    # open, so the contender can prove exclusion but not read the owner label.
    return "unknown" if os.name == "nt" else owner


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
                    args=(tmp, output),
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
                            owner_kind="desktop",
                            generation="desktop",
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
                Path(account) / ".kassiber" / "run" / "operator-owners",
            )

    def test_broker_first_blocks_desktop_owner(self) -> None:
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
                with self.assertRaises(AppError) as raised:
                    daemon_runtime._ensure_daemon_project_owner(ctx)
                self.assertEqual(raised.exception.code, "project_in_use")
                self.assertEqual(
                    raised.exception.details["owner"],
                    _reported_locked_owner("broker"),
                )
            finally:
                broker.release()

    def test_desktop_first_blocks_broker_unlock(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.operator.service.open_db", return_value=_Connection()
        ) as open_database:
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
                with self.assertRaises(AppError) as raised:
                    service.unlock(tmp, bytearray(b"passphrase"), duration_seconds=None)
                self.assertEqual(raised.exception.code, "project_in_use")
                self.assertEqual(
                    raised.exception.details["owner"],
                    _reported_locked_owner("desktop"),
                )
                open_database.assert_not_called()
            finally:
                service.close()
                desktop.release()

    def test_broker_owner_blocks_daemon_before_database_open(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, mock.patch(
            "kassiber.daemon.open_db", return_value=_Connection()
        ) as open_database:
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
            )
            try:
                with self.assertRaises(AppError) as raised:
                    daemon_runtime._open_daemon_connection(ctx)
                self.assertEqual(raised.exception.code, "project_in_use")
                open_database.assert_not_called()
            finally:
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
                        owner_kind="desktop",
                        generation="desktop",
                    )
                self.assertEqual(raised.exception.code, "project_in_use")
            finally:
                child.wait(timeout=2)
            next_owner = acquire_project_ownership(
                project,
                owner_kind="desktop",
                generation="desktop",
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
                        owner_kind="desktop",
                        generation="desktop",
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

    def test_windows_owner_contract_uses_inheritable_share_mode_reservation(self) -> None:
        source = (
            Path(__file__).parents[1] / "kassiber" / "operator" / "project.py"
        ).read_text(encoding="utf-8")
        self.assertIn("CreateFileW", source)
        self.assertIn("no sharing", source)
        self.assertIn("os.dup", source)
        self.assertIn("os.set_handle_inheritable", source)


if __name__ == "__main__":
    unittest.main()
