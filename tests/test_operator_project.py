from __future__ import annotations

import multiprocessing
import os
import tempfile
import unittest
from pathlib import Path

from kassiber.errors import AppError
from kassiber.operator.project import (
    OWNER_LOCK_FILENAME,
    acquire_project_ownership,
    canonical_project,
)


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


class OperatorProjectTest(unittest.TestCase):
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
                output: multiprocessing.Queue = multiprocessing.Queue()
                process = multiprocessing.Process(
                    target=_competing_owner,
                    args=(tmp, output),
                )
                process.start()
                process.join(5)
                self.assertEqual(process.exitcode, 0)
                code, details = output.get(timeout=1)
                self.assertEqual(code, "project_in_use")
                self.assertEqual(details["project"], project.public_id)
                self.assertEqual(details["owner"], "broker")
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
                self.assertEqual(
                    Path(tmp, OWNER_LOCK_FILENAME).stat().st_mode & 0o777,
                    0o600,
                )


if __name__ == "__main__":
    unittest.main()
