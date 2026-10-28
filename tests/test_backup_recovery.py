"""A killed restore must recover before ordinary database initialization."""
import os
from pathlib import Path
import subprocess
import sys

import pytest

from kassiber.backup import install
from kassiber.db import open_db
from kassiber.operator import project


def test_restart_after_live_database_move_recovers_original_before_open(tmp_path, monkeypatch):
    data = tmp_path / "book/data"
    conn = open_db(data)
    conn.execute("CREATE TABLE restore_regression(value)")
    conn.execute("INSERT INTO restore_regression VALUES('original book')")
    conn.commit()
    conn.close()
    stage = tmp_path / "stage"
    stage.mkdir()
    (stage / "kassiber.sqlite3").write_bytes(b"incoming book")
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    monkeypatch.setattr(project, "_owner_lock_root", lambda: owners)
    script = """
import os, sys
from pathlib import Path
from kassiber.backup import install
from kassiber.operator import project
data, stage, owners = map(Path, sys.argv[1:])
project._owner_lock_root = lambda: owners
original = os.replace
def replace(source, target):
    original(source, target)
    if Path(source) == data / 'kassiber.sqlite3':
        os._exit(71)
install.os.replace = replace
with project.exclusive_project_maintenance(data, active_owner_kind=None):
    install.install_staged_backup(stage, data)
"""
    killed = subprocess.run([sys.executable, "-c", script, str(data), str(stage), str(owners)],
                            env={**os.environ, "KASSIBER_NO_EGRESS": "1"}, check=False)
    assert killed.returncode == 71
    conn = open_db(data)
    try:
        assert conn.execute("SELECT value FROM restore_regression").fetchone()[0] == "original book"
    finally:
        conn.close()


def test_every_install_error_preserves_original_or_committed_tree(tmp_path):
    def setup(root):
        data, stage = root / "book/data", root / "stage"
        data.mkdir(parents=True)
        stage.mkdir()
        for name, target in install.restore_targets(data).items():
            target.parent.mkdir(parents=True, exist_ok=True)
            if name in ("attachments", "exports"):
                target.mkdir()
                target /= "old"
            target.write_bytes(b"old " + name.encode())
        for name in install.INCOMING:
            target = stage / name
            target.parent.mkdir(parents=True, exist_ok=True)
            if name == "attachments":
                target.mkdir()
                target /= "new"
            target.write_bytes(b"new " + name.encode())
        return data, stage

    data, stage = setup(tmp_path / "trace")
    points = []
    install.install_staged_backup(stage, data, _observer=points.append)
    for index, point in enumerate(points):
        data, stage = setup(tmp_path / str(index))
        def fail(observed):
            if observed == point:
                raise OSError("injected durable boundary")
        try:
            install.install_staged_backup(stage, data, _observer=fail)
        except install.AppError as failure:
            assert failure.code == "restore_install_failed", (point, failure)
        committed = index >= points.index("restore.committed.journal_replace")
        expected = b"new kassiber.sqlite3" if committed else b"old kassiber.sqlite3"
        assert (data / "kassiber.sqlite3").read_bytes() == expected, point
        assert not (data.parent / install.JOURNAL).exists(), point
        if committed:
            recovery, = data.parent.glob("pre-restore-*")
            for name in install.NAMES:
                saved = recovery / name
                if name in ("attachments", "exports"):
                    saved /= "old"
                assert saved.read_bytes() == b"old " + name.encode(), point
            assert not (data / "kassiber.sqlite3-journal").exists(), point
            assert not (data.parent / "exports").exists(), point
        else:
            for name, target in install.restore_targets(data).items():
                if name in ("attachments", "exports"):
                    target /= "old"
                assert target.read_bytes() == b"old " + name.encode(), point


def test_malformed_or_symlinked_recovery_intent_never_changes_live_files(tmp_path):
    data = tmp_path / "book/data"
    data.mkdir(parents=True)
    live = data / "kassiber.sqlite3"
    live.write_bytes(b"retained")
    journal = data.parent / install.JOURNAL
    for content in (b"{", b"[]", b'{"prepared":"../outside"}', b"x" * 65537):
        journal.write_bytes(content)
        with pytest.raises(install.AppError) as failure:
            install.recover_staged_restore(data)
        assert failure.value.code == "restore_recovery_required"
        assert live.read_bytes() == b"retained"
    journal.unlink()
    journal.symlink_to(live)
    with pytest.raises(install.AppError) as failure:
        install.recover_staged_restore(data)
    assert failure.value.code == "unsafe_restore_target"
    assert live.read_bytes() == b"retained"
