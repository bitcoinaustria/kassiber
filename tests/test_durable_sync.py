"""Full-sync durability on macOS, checked on any platform through a seam.

`kassiber.durable_sync._platform` is replaced with "darwin" and its `fcntl`
with a recorder, so the macOS branch runs on Linux: every `F_FULLFSYNC` is
recorded with the path its descriptor points at, and no real device flush is
requested. The SQLite pragmas are plain per-connection flags that every
SQLite build accepts, so their values read back on any platform.
"""

from __future__ import annotations

import os
import sqlite3
import sys
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import pytest

from kassiber import durable_sync, private_files, projects
from kassiber.db import open_db, resolve_settings_path, update_managed_settings
from kassiber.secrets import migration, sqlcipher

try:
    import fcntl as real_fcntl
except ImportError:  # Windows
    real_fcntl = None

F_FULLFSYNC = 51  # the macOS value; the recorder only compares it


def _fd_path(fd: int) -> Path:
    if Path("/proc/self/fd").is_dir():
        return Path(os.readlink(f"/proc/self/fd/{fd}"))
    if real_fcntl is not None and hasattr(real_fcntl, "F_GETPATH"):
        raw = real_fcntl.fcntl(fd, real_fcntl.F_GETPATH, bytes(1024))
        return Path(raw.split(b"\0", 1)[0].decode())
    pytest.skip("no way to map a descriptor to its path on this platform")


class FullSyncRecorder:
    def __init__(self, *, refuse: bool = False):
        self.paths: list[Path] = []
        self.plain: list[Path] = []
        self.refuse = refuse

    def fcntl(self, fd, command, *args):
        assert command == F_FULLFSYNC
        if self.refuse:
            raise OSError(45, "Operation not supported")
        self.paths.append(_fd_path(fd))
        return 0


@pytest.fixture
def macos(monkeypatch):
    recorder = FullSyncRecorder()
    monkeypatch.setattr(durable_sync, "_platform", lambda: "darwin")
    monkeypatch.setattr(durable_sync, "fcntl", SimpleNamespace(F_FULLFSYNC=F_FULLFSYNC, fcntl=recorder.fcntl))
    real_fsync = os.fsync

    def plain(fd):
        recorder.plain.append(_fd_path(fd))
        real_fsync(fd)

    monkeypatch.setattr(durable_sync.os, "fsync", plain)
    return recorder


def _resolved(path) -> Path:
    return Path(path).resolve()


def test_full_fsync_uses_f_fullfsync_on_macos_only(tmp_path, macos, monkeypatch):
    target = tmp_path / "file"
    target.write_bytes(b"x")
    durable_sync.sync_file(target)
    assert macos.paths == [_resolved(target)] and macos.plain == []

    monkeypatch.setattr(durable_sync, "_platform", lambda: "linux")
    durable_sync.sync_file(target)
    assert macos.paths == [_resolved(target)] and macos.plain == [_resolved(target)]


def test_refused_full_fsync_falls_back_to_fsync(tmp_path, macos):
    macos.refuse = True
    target = tmp_path / "file"
    target.write_bytes(b"x")
    durable_sync.sync_file(target)
    assert macos.plain == [_resolved(target)]


@pytest.mark.skipif(os.name == "nt", reason="directory descriptors are POSIX")
def test_directory_and_tree_syncs_reach_every_level(tmp_path, macos):
    (tmp_path / "a/b").mkdir(parents=True)
    (tmp_path / "a/b/leaf").write_bytes(b"1")
    (tmp_path / "top").write_bytes(b"2")
    durable_sync.sync_tree(tmp_path)
    root = _resolved(tmp_path)
    assert set(macos.paths) == {root, root / "a", root / "a/b", root / "a/b/leaf", root / "top"}
    # Files first, then directories from the deepest up to the root.
    assert macos.paths[-3:] == [root / "a/b", root / "a", root]


def _durability(conn) -> tuple[int, int]:
    return (
        conn.execute("PRAGMA fullfsync").fetchone()[0],
        conn.execute("PRAGMA checkpoint_fullfsync").fetchone()[0],
    )


def test_book_connections_set_fullfsync_pragmas_on_macos(tmp_path, macos):
    conn = open_db(tmp_path / "mac")
    try:
        assert _durability(conn) == (1, 1)
    finally:
        conn.close()


def test_book_connections_keep_sqlite_defaults_elsewhere(tmp_path, monkeypatch):
    monkeypatch.setattr(durable_sync, "_platform", lambda: "linux")
    conn = open_db(tmp_path / "linux")
    try:
        assert _durability(conn) == (0, 0)
    finally:
        conn.close()


def test_keyed_sqlcipher_connections_set_fullfsync_pragmas_on_macos(tmp_path, macos):
    pytest.importorskip("sqlcipher3")
    database = migration.create_empty_encrypted_database(tmp_path / "book.sqlite3", "Synthetic passphrase 1")
    assert _resolved(database) in macos.paths
    assert _resolved(tmp_path) in macos.paths
    conn = sqlcipher.open_encrypted(database, "Synthetic passphrase 1", enforce_operator_identity=False)
    try:
        assert _durability(conn) == (1, 1)
    finally:
        conn.close()


def test_sqlcipher_migration_syncs_the_copy_before_publishing_it(tmp_path, macos, monkeypatch):
    pytest.importorskip("sqlcipher3")
    source = tmp_path / "kassiber.sqlite3"
    conn = sqlite3.connect(source)
    conn.execute("CREATE TABLE t(x)")
    conn.commit()
    conn.close()
    pragmas = []
    real = migration.apply_sqlite_durability

    def spy(connection):
        real(connection)
        pragmas.append(_durability(connection))

    monkeypatch.setattr(migration, "apply_sqlite_durability", spy)
    renames = []
    real_rename = Path.rename

    def rename(self, target):
        renames.append((len(macos.paths), _resolved(self)))
        return real_rename(self, target)

    monkeypatch.setattr(Path, "rename", rename)
    result = migration.migrate_plaintext_to_encrypted(source, "Synthetic passphrase 1", credential_markers=())
    assert pragmas == [(1, 1)]
    candidate = _resolved(tmp_path / ("kassiber" + migration.ENCRYPTED_SUFFIX))
    before_publish = macos.paths[: renames[0][0]]
    assert before_publish[-2:] == [candidate, _resolved(tmp_path)]
    assert len(renames) == 2
    assert macos.paths[renames[-1][0]:] == [_resolved(tmp_path)]
    assert result.encrypted_path == source


@pytest.mark.skipif(os.name == "nt", reason="directory descriptors are POSIX")
def test_private_preference_and_settings_sync_their_directory(tmp_path, macos):
    private_files.atomic_write_private(tmp_path / "config/offline-mode.json", "{}\n")
    assert macos.paths[-1] == _resolved(tmp_path / "config")
    data_root = tmp_path / "data"
    data_root.mkdir()
    update_managed_settings(data_root, updates={"synthetic": True})
    settings = resolve_settings_path(data_root)
    assert settings.exists()
    assert macos.paths[-1] == _resolved(settings.parent)


@pytest.mark.skipif(os.name == "nt", reason="directory descriptors are POSIX")
def test_project_catalog_is_synced_before_and_after_its_rename(tmp_path, macos):
    catalog = tmp_path / "config/projects.json"
    projects.write_catalog({"projects": []}, catalog)
    temporaries = [path for path in macos.paths if path.name.startswith("projects.json.") and path.name.endswith(".tmp")]
    assert len(temporaries) == 1
    assert macos.paths[-1] == _resolved(catalog.parent)


@pytest.mark.skipif(os.name == "nt", reason="directory descriptors are POSIX")
def test_legacy_staging_copy_is_synced_before_it_is_published(tmp_path, macos, monkeypatch):
    state_root = tmp_path / ".kassiber"
    data_root = state_root / "data"
    data_root.mkdir(parents=True)
    (state_root / "attachments").mkdir()
    (state_root / "attachments/evidence.txt").write_text("receipt", encoding="utf-8")
    conn = sqlite3.connect(data_root / projects.DEFAULT_DB_FILENAME)
    conn.execute("CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT, created_at TEXT)")
    conn.commit()
    conn.close()
    monkeypatch.setattr(projects, "DEFAULT_STATE_ROOT", str(state_root))
    monkeypatch.setattr(projects, "DEFAULT_DATA_ROOT", str(data_root))
    assert projects.migrate_legacy_default_layout_if_needed().id == "default"
    staging = _resolved(state_root / "projects") / "default.migrating"
    staged = {path.relative_to(staging).as_posix() for path in macos.paths if staging in path.parents or path == staging}
    assert {"data/" + projects.DEFAULT_DB_FILENAME, "attachments/evidence.txt", "data", "attachments", "."} <= staged
    published = macos.paths.index(_resolved(state_root / "projects"))
    assert all(staging not in path.parents and path != staging for path in macos.paths[published:])


def test_the_seam_is_the_real_platform_by_default():
    assert durable_sync._platform() == sys.platform


@pytest.mark.skipif(os.name == "nt", reason="directory descriptors are POSIX")
def test_backup_export_syncs_the_archive_and_its_directory(tmp_path, macos):
    from kassiber.backup import pack
    from kassiber.backup.age_cli import AgeBackend

    data_root = tmp_path / "data"
    data_root.mkdir()
    (data_root / "kassiber.sqlite3").write_bytes(b"live-db")
    output = tmp_path / "out/snap.kassiber"
    output.parent.mkdir()

    def copy(_source, _passphrase, destination):
        destination.write_bytes(b"encrypted-db-copy")

    def encrypt(_source, destination, **_kwargs):
        destination.write(b"ciphertext")

    with mock.patch.object(pack, "_backup_sqlcipher_database", side_effect=copy), mock.patch.object(
        pack, "_collect_ai_provider_secret_refs", return_value=[]
    ), mock.patch.object(pack, "encrypt_age_stream", side_effect=encrypt):
        pack.export_backup(str(data_root), output, "db-pass", backup_passphrase="outer", age_backend=AgeBackend(flavor="pyrage"))
    assert output.read_bytes() == b"ciphertext"
    archive = [path for path in macos.paths if path.name.startswith(".snap.kassiber.")]
    assert len(archive) == 1
    assert macos.paths[-1] == _resolved(output.parent)


def test_the_host_platform_decides_without_the_seam(tmp_path):
    """Runs the real `F_FULLFSYNC` and pragmas on macOS (the Mac agent's check)."""
    target = tmp_path / "file"
    target.write_bytes(b"x")
    durable_sync.sync_file(target)
    if os.name != "nt":
        durable_sync.sync_directory(tmp_path)
    conn = open_db(tmp_path / "host")
    try:
        assert _durability(conn) == ((1, 1) if sys.platform == "darwin" else (0, 0))
    finally:
        conn.close()

