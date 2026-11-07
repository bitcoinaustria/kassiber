"""The test run's owner-lock sweep obeys the product's admission protocol.

Pinned here: a group whose admission lock another process holds (an acquirer
between opening its record and locking it) is left whole; a held record stays
while its unheld siblings go; admission files are never removed; groups not
touched since the session started are not looked at; and only the process
that started the run sweeps, never an xdist worker that drained early.
"""

from __future__ import annotations

import os
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace

from kassiber.operator import project as project_module
from tests import conftest
from tests.owner_lock_sweep import sweep_owner_locks


@unittest.skipIf(os.name == "nt", "the sweep does not run on Windows")
class OwnerLockSweepTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="owner-lock-sweep-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.started = time.time() - 5.0

    def group(self, name):
        key = self.root / f"{name}.lock"
        paths = [key, *(key.with_name(f"{key.name}.{suffix}") for suffix in ("desktop", "broker", "admission"))]
        for path in paths:
            path.write_bytes(b"")
        return key, paths

    def hold(self, path):
        """Lock `path` through its own open file description, as another process would."""
        handle = project_module._open_owner_lock(path, "test")
        self.addCleanup(handle.close)
        self.assertTrue(project_module._try_lock_handle(handle))
        return handle

    def names(self):
        return sorted(path.name for path in self.root.iterdir())

    def test_dead_records_go_and_the_admission_file_stays(self):
        key, _ = self.group("identity-a")
        removed = sweep_owner_locks(self.root, self.started)
        self.assertEqual(sorted(path.name for path in removed),
                         ["identity-a.lock", "identity-a.lock.broker", "identity-a.lock.desktop"])
        self.assertEqual(self.names(), ["identity-a.lock.admission"])

    def test_a_group_mid_acquire_is_left_whole(self):
        key, _ = self.group("identity-a")
        before = self.names()
        admission = self.hold(project_module._owner_admission_lock_path(key))
        self.assertEqual(sweep_owner_locks(self.root, self.started), [])
        self.assertEqual(self.names(), before)
        project_module._unlock_handle(admission)
        sweep_owner_locks(self.root, self.started)
        self.assertEqual(self.names(), ["identity-a.lock.admission"])

    def test_a_held_record_stays(self):
        key, _ = self.group("path-b")
        self.hold(key.with_name(f"{key.name}.desktop"))
        sweep_owner_locks(self.root, self.started)
        self.assertEqual(self.names(), ["path-b.lock.admission", "path-b.lock.desktop"])

    def test_groups_untouched_since_the_start_are_not_swept(self):
        _, old = self.group("identity-old")
        for path in old:
            os.utime(path, (self.started - 60, self.started - 60))
        self.group("identity-new")
        sweep_owner_locks(self.root, self.started)
        self.assertEqual(self.names(), [
            "identity-new.lock.admission",
            "identity-old.lock", "identity-old.lock.admission",
            "identity-old.lock.broker", "identity-old.lock.desktop",
        ])

    def test_only_the_process_that_started_the_run_sweeps(self):
        previous = conftest._OWNER_LOCK_SWEEP_STARTED
        self.addCleanup(setattr, conftest, "_OWNER_LOCK_SWEEP_STARTED", previous)
        conftest._OWNER_LOCK_SWEEP_STARTED = None
        conftest._start_owner_lock_sweep(SimpleNamespace(workerinput={"workerid": "gw0"}))
        self.assertIsNone(conftest._OWNER_LOCK_SWEEP_STARTED)
        conftest._start_owner_lock_sweep(SimpleNamespace())
        self.assertIsNotNone(conftest._OWNER_LOCK_SWEEP_STARTED)


if __name__ == "__main__":
    unittest.main()
