"""Acceptance prerequisite for R3 legacy project migration.

Python is the oracle. Before introducing a Rust publication protocol, require
Python recovery to finish the same work after the legacy database is moved.
The strict xfails record the reference blockers, not Rust implementation parity.
Run with --runxfail to expose the unmet acceptance requirements. Logical dumps
and non-database tree bytes are checked separately from physical database bytes.
"""
from __future__ import annotations

from contextlib import contextmanager
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import select
import shutil
import sqlite3
import stat
import subprocess
import sys
from unittest.mock import patch

import pytest

from kassiber import db, projects
from kassiber.operator import project as ownership
from kassiber.secrets import sqlcipher
from tests.test_rust_store_migrations import PASSPHRASE as KEY, normalized_dump, reference_open


STAMP = "2026-10-27T12:34:56Z"
ROOT = Path(__file__).resolve().parents[1]


class _ReferenceContractMismatch(Exception):
    """Only the demonstrated acceptance mismatch may be an expected failure."""


class _Clock(datetime):
    @classmethod
    def now(cls, tz=None):
        value = datetime(2026, 10, 27, 12, 34, 56, tzinfo=timezone.utc)
        return value.astimezone(tz) if tz else value.replace(tzinfo=None)


@contextmanager
def _reference_paths(root):
    """Patch imported legacy constants too; never consult an account catalog."""
    state = root / "state"
    xdg = root / "xdg-flat"
    satbooks = root / "satbooks-flat"
    with (
        patch.multiple(projects, DEFAULT_STATE_ROOT=str(state),
                       DEFAULT_DATA_ROOT=str(state / "data"),
                       LEGACY_XDG_DATA_ROOT=str(xdg), LEGACY_DATA_ROOT=str(satbooks)),
        patch.multiple(db, DEFAULT_STATE_ROOT=str(state),
                       DEFAULT_DATA_ROOT=str(state / "data"),
                       LEGACY_XDG_DATA_ROOT=str(xdg), LEGACY_DATA_ROOT=str(satbooks)),
        patch.object(db, "default_state_root", return_value=state),
        patch.object(projects, "now_iso", return_value=STAMP),
        patch.object(projects, "datetime", _Clock),
    ):
        yield


@pytest.fixture(scope="module", autouse=True)
def _sweep_session_owner_locks(tmp_path_factory):
    owners = tmp_path_factory.mktemp("r3b-reference-owners")
    owners.chmod(0o700)
    with patch.object(ownership, "_owner_lock_root", return_value=owners):
        yield


@pytest.fixture(autouse=True)
def _disposable_environment(tmp_path, monkeypatch):
    home = tmp_path / "home"
    home.mkdir(mode=0o700)
    for name in ("HOME", "USERPROFILE"):
        monkeypatch.setenv(name, str(home))
    for name in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
        monkeypatch.setenv(name, str(home / name.lower()))
    for name in ("KASSIBER_OPERATOR_CHILD", "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
                 "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY"):
        monkeypatch.delenv(name, raising=False)


def _seed(root, encrypted, layout, workspace_count):
    state = root / "state"
    data = {"xdg": root / "xdg-flat", "satbooks": root / "satbooks-flat"}.get(layout, state / "data")
    source_state = data if layout in {"xdg", "satbooks"} else state
    data.mkdir(parents=True)
    connection = reference_open(data, encrypted)
    for index in range(workspace_count):
        connection.execute("INSERT INTO workspaces(id,label,created_at) VALUES(?,?,?)",
                           (f"workspace-{index}", f"Book {index}", STAMP))
    connection.commit()
    connection.close()
    if layout == "satbooks":
        (data / "kassiber.sqlite3").rename(data / "satbooks.sqlite3")
    for relative, contents in {
        "attachments/evidence.txt": b"synthetic evidence\x00\xff",
        "exports/report.csv": b"amount_msat\n100000000001\n",
        "config/backends.env": b"ELECTRUM_HOST=synthetic.invalid\n",
        "config/settings.json": b'{"appearance": "dark"}\n',
    }.items():
        path = source_state / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(contents)
    (source_state / "attachments/empty").mkdir()
    if layout == "stale":
        stale = state / "projects/default.migrating"
        stale.mkdir(parents=True)
        (stale / "partial.txt").write_text("interrupted", encoding="utf-8")
    if layout == "promoted":
        target = state / "projects/default/data"
        target.mkdir(parents=True)
        shutil.copy2(data / "kassiber.sqlite3", target / "kassiber.sqlite3")
    # Use private disposable inputs; still compare every final mode exactly.
    for path in root.rglob("*"):
        path.chmod(0o700 if path.is_dir() else 0o600)


def _finish(root, encrypted):
    with _reference_paths(root):
        entry = projects.ensure_default_project()
        connection = reference_open(entry.data_root, encrypted)
        projects.validate_project_migration_after_unlock(entry.data_root, connection)
        connection.close()
        return entry


def _tree(root):
    """No path, byte, mode or database-file masks."""
    return {
        str(path.relative_to(root)): (
            stat.S_IMODE(path.lstat().st_mode),
            None if path.is_dir() else path.read_bytes(),
        )
        for path in sorted(root.rglob("*"))
    }


def _logical_tree(tree):
    # Explicit logical comparison, separate from the physical-byte acceptance.
    # Keep every filename and mode, and every non-database file's bytes.
    return {name: (mode, None if name.endswith(".sqlite3") else contents)
            for name, (mode, contents) in tree.items()}


def _databases(root, encrypted):
    output = {}
    for path in sorted(root.rglob("*.sqlite3")):
        connection = (sqlcipher.open_encrypted(path, KEY, enforce_operator_identity=False)
                      if encrypted else sqlite3.connect(path))
        try:
            assert connection.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
            assert connection.execute("PRAGMA foreign_key_check").fetchall() == []
            output[str(path.relative_to(root))] = normalized_dump(connection)
        finally:
            connection.close()
    return output


@pytest.mark.parametrize("layout", ["flat", "xdg", "satbooks", "stale", "promoted"])
@pytest.mark.parametrize("workspace_count", [1, 2], ids=["single", "multi"])
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_python_legacy_layout_keeps_workspaces_and_reopens_unchanged(tmp_path, encrypted, layout, workspace_count):
    root = tmp_path / "case"
    _seed(root, encrypted, layout, workspace_count)
    entry = _finish(root, encrypted)
    assert entry.id == "default"
    assert entry.encrypted == encrypted
    before_tree = _tree(root)
    before_dumps = _databases(root, encrypted)
    assert len(before_dumps) == 2  # Published database plus retained original.
    for _, tables in before_dumps.values():
        assert len(tables["workspaces"][1]) == workspace_count
    catalog = json.loads((root / "state/config/projects.json").read_bytes())
    assert catalog["selected_project_id"] == "default"
    assert len(catalog["projects"]) == 1
    assert _finish(root, encrypted) == entry
    assert _databases(root, encrypted) == before_dumps
    assert _logical_tree(_tree(root)) == _logical_tree(before_tree)


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.xfail(strict=True, raises=_ReferenceContractMismatch,
                   reason="Python open_db changes physical database bytes despite identical normalized dumps")
def test_python_reopen_preserves_physical_database_bytes(tmp_path, encrypted):
    root = tmp_path / "case"
    _seed(root, encrypted, "flat", 2)
    _finish(root, encrypted)
    before_tree = _tree(root)
    before_dumps = _databases(root, encrypted)
    _finish(root, encrypted)
    assert _databases(root, encrypted) == before_dumps
    after_tree = _tree(root)
    assert _logical_tree(after_tree) == _logical_tree(before_tree)
    if after_tree != before_tree:
        changed = [name for name in before_tree if before_tree[name] != after_tree[name]]
        raise _ReferenceContractMismatch(f"Python reopen rewrote database bytes: {changed}")


_CHILD = """
import json
from pathlib import Path
import sys
from unittest.mock import patch
from tests.test_rust_store_partition_reference import _reference_paths
from kassiber import projects

root = Path(sys.argv[1])
original = projects._move_if_exists
def move(source, backup_root, source_state):
    result = original(source, backup_root, source_state)
    if source.name == 'kassiber.sqlite3':
        print(json.dumps({'checkpoint': 'legacy.database_moved'}), flush=True)
        sys.stdin.readline()
    return result
with _reference_paths(root), patch.object(projects, '_move_if_exists', side_effect=move):
    projects.ensure_default_project()
"""


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child pipe harness")
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.xfail(strict=True, raises=_ReferenceContractMismatch,
                   reason="Python discovers recovery only through the now-moved legacy database; cleanup and marker remain unfinished")
def test_python_resume_after_database_move_matches_complete_migration(tmp_path, encrypted):
    root, seed = tmp_path / "case", tmp_path / "seed"
    _seed(root, encrypted, "flat", 2)
    shutil.copytree(root, seed, copy_function=shutil.copy2)
    _finish(root, encrypted)
    expected_tree = _tree(root)
    expected_dumps = _databases(root, encrypted)
    shutil.rmtree(root)
    shutil.copytree(seed, root, copy_function=shutil.copy2)
    child = subprocess.Popen(
        [sys.executable, "-c", _CHILD, str(root)], cwd=ROOT,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        assert select.select([child.stdout], [], [], 30)[0], "child did not reach publication boundary"
        assert child.stdout.readline().strip() == '{"checkpoint": "legacy.database_moved"}'
        child.kill()
        _, stderr = child.communicate(timeout=30)
        assert child.returncode < 0, stderr
    finally:
        if child.poll() is None:
            child.kill()
            child.communicate(timeout=30)
    # Ordinary Python resolution/open succeeds, but cannot finish the migration.
    _finish(root, encrypted)
    assert _databases(root, encrypted) == expected_dumps
    # Prove the independent recovery failure without conflating SQLCipher IVs
    # or SQLite change counters with the unfinished non-database artifacts.
    actual_tree = _logical_tree(_tree(root))
    expected_tree = _logical_tree(expected_tree)
    differences = [name for name in sorted(expected_tree.keys() | actual_tree.keys())
                   if expected_tree.get(name) != actual_tree.get(name)]
    if differences:
        raise _ReferenceContractMismatch(f"Python recovery left an unfinished tree: {differences}")
