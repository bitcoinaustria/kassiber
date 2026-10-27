"""R3 legacy project acceptance: Python reference and private Rust parity.

Compare complete normalized database dumps and byte-exact non-database files,
including modes. Physical SQLite counters and SQLCipher pages are not an oracle.
All paths, clocks, identities and ownership namespaces are disposable/injected.
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
from tests.test_rust_store_migrations import (PASSPHRASE as KEY, normalized_dump, reference_open,
                                               storage_probe, invoke)


STAMP = "2026-10-27T12:34:56Z"
ROOT = Path(__file__).resolve().parents[1]


class _Clock(datetime):
    @classmethod
    def now(cls, tz=None):
        value = datetime(2026, 10, 27, 12, 34, 56, tzinfo=timezone.utc)
        return value.astimezone(tz) if tz else value.replace(tzinfo=None)


@contextmanager
def _reference_paths(root):
    """Patch imported legacy constants too; never consult an account catalog."""
    state = root / "state/kassiber"
    xdg = root / "home/.local/share/kassiber"
    satbooks = root / "home/.local/share/satbooks"
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
    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    for name in ("HOME", "USERPROFILE"):
        monkeypatch.setenv(name, str(home))
    for name in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
        monkeypatch.setenv(name, str(home / name.lower()))
    for name in ("KASSIBER_OPERATOR_CHILD", "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
                 "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY"):
        monkeypatch.delenv(name, raising=False)


def _seed(root, encrypted, layout, workspace_count):
    state = root / "state/kassiber"
    data = {"xdg": root / "home/.local/share/kassiber", "satbooks": root / "home/.local/share/satbooks"}.get(layout, state / "data")
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
    """Capture all paths, modes and raw bytes before the declared DB boundary."""
    return {
        str(path.relative_to(root)): (
            stat.S_IMODE(path.lstat().st_mode),
            None if path.is_dir() else path.read_bytes(),
        )
        for path in sorted(root.rglob("*"))
    }


def _logical_tree(tree):
    # Database contents are compared by complete normalized dumps below.
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
    catalog = json.loads((root / "state/kassiber/config/projects.json").read_bytes())
    assert catalog["selected_project_id"] == "default"
    assert len(catalog["projects"]) == 1
    assert _finish(root, encrypted) == entry
    assert _databases(root, encrypted) == before_dumps
    assert _logical_tree(_tree(root)) == _logical_tree(before_tree)


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_python_reopen_preserves_complete_database_dump(tmp_path, encrypted):
    root = tmp_path / "case"
    _seed(root, encrypted, "flat", 2)
    _finish(root, encrypted)
    before_tree = _tree(root)
    before_dumps = _databases(root, encrypted)
    _finish(root, encrypted)
    assert _databases(root, encrypted) == before_dumps
    after_tree = _tree(root)
    assert _logical_tree(after_tree) == _logical_tree(before_tree)


def _context(root):
    return dict(home=str(root / "home"), platform="linux",
                environ={"XDG_DATA_HOME": str(root / "state")}, timestamp=STAMP)


def _finish_rust(binary, root, encrypted, **options):
    result = invoke(binary, root / "unused", encrypted=encrypted,
                    op="project-migration", **_context(root), **options)
    assert result["ok"], result
    return result["data"]["project"]


def test_policy_inventory_matches_python():
    assert json.loads((ROOT / "crates/store/fixtures/project-policy.json").read_bytes()) == projects.WORKSPACE_SPLIT_POLICY


@pytest.mark.parametrize("layout", ["flat", "xdg", "satbooks", "stale", "promoted"])
@pytest.mark.parametrize("workspace_count", [1, 2], ids=["single", "multi"])
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_rust_legacy_layout_matches_complete_python_result(storage_probe, tmp_path, encrypted, layout, workspace_count):
    # Restore one exact seed at the same absolute paths. No path/time masking.
    root, seed = tmp_path / "case", tmp_path / "seed"
    _seed(root, encrypted, layout, workspace_count)
    shutil.copytree(root, seed, copy_function=shutil.copy2)
    _finish(root, encrypted)
    expected_tree, expected_dumps = _logical_tree(_tree(root)), _databases(root, encrypted)
    shutil.rmtree(root)
    shutil.copytree(seed, root, copy_function=shutil.copy2)
    entry = _finish_rust(storage_probe, root, encrypted)
    assert entry["id"] == "default"
    assert entry["encrypted"] == encrypted
    assert _databases(root, encrypted) == expected_dumps
    assert _logical_tree(_tree(root)) == expected_tree
    _finish(root, encrypted)
    _finish_rust(storage_probe, root, encrypted)
    assert _databases(root, encrypted) == expected_dumps
    assert _logical_tree(_tree(root)) == expected_tree


def _python_child(root, encrypted, checkpoint):
    def stop(label):
        if label == checkpoint:
            print(json.dumps({"checkpoint": label}), flush=True)
            sys.stdin.readline()
    move = projects._move_if_exists
    copy = projects._copy_if_exists
    copy2 = projects.shutil.copy2
    write = projects._write_migration_json
    report = projects._write_migration_report
    replace = Path.replace
    create = projects.create_project
    keys = {"attachments": "attachments", "exports": "exports", "backends.env": "backends_env",
            "settings.json": "settings_json", "kassiber.sqlite3": "database", "satbooks.sqlite3": "database"}
    def moving(source, backup, state):
        key = keys[source.name]
        stop("legacy.before_move." + key)
        result = move(source, backup, state)
        stop("legacy.after_move." + key)
        return result
    def copying(source, target):
        result = copy(source, target)
        stop("legacy.copy." + keys[source.name])
        return result
    def copying2(source, target, **kwargs):
        database = Path(target).name == "kassiber.sqlite3" and "default.migrating" in Path(target).parts
        if database:
            stop("legacy.staging")
        result = copy2(source, target, **kwargs)
        if database:
            stop("legacy.copy.database")
        return result
    def writing(path, marker, **kwargs):
        write(path, marker, **kwargs)
        if "pending_legacy_artifacts" in marker:
            stop("legacy.intent")
        elif "validated_at" in marker:
            stop("legacy.validated")
        elif "legacy_artifacts_moved_at" in marker:
            stop("legacy.complete")
        elif "report" not in marker:
            stop("legacy.marker")
    def reporting(**kwargs):
        result = report(**kwargs)
        stop("legacy.unlock_report" if kwargs["status"].startswith("validated") else "legacy.report")
        return result
    def replacing(source, target):
        if source.name == "default.migrating":
            stop("legacy.before_promote")
        result = replace(source, target)
        if source.name == "default.migrating":
            stop("legacy.after_promote")
        return result
    def creating(*args, **kwargs):
        result = create(*args, **kwargs)
        stop("legacy.catalog")
        return result
    with (_reference_paths(root),
          patch.object(projects, "_move_if_exists", moving),
          patch.object(projects, "_copy_if_exists", copying),
          patch.object(projects.shutil, "copy2", copying2),
          patch.object(projects, "_write_migration_json", writing),
          patch.object(projects, "_write_migration_report", reporting),
          patch.object(Path, "replace", replacing),
          patch.object(projects, "create_project", creating)):
        _finish(root, encrypted)


_CHILD = """
import sys
from pathlib import Path
from tests.test_rust_store_partition_reference import _python_child
_python_child(Path(sys.argv[1]), sys.argv[2] == 'True', sys.argv[3])
"""


MOVE_POINTS = [f"legacy.{when}_move.{key}" for key in
               ["attachments", "exports", "backends_env", "settings_json", "database"]
               for when in ["before", "after"]]
PUBLICATION_POINTS = ["legacy.staging", "legacy.copy.database", "legacy.copy.attachments",
                      "legacy.copy.exports", "legacy.copy.backends_env", "legacy.copy.settings_json",
                      "legacy.marker", "legacy.report", "legacy.before_promote", "legacy.after_promote",
                      "legacy.intent", "legacy.complete", "legacy.catalog"]


def _kill_at(binary, runtime, root, encrypted, checkpoint):
    if runtime == "python":
        command = [sys.executable, "-c", _CHILD, str(root), str(encrypted), checkpoint]
        request = None
    else:
        command = [str(binary)]
        request = dict(op="project-migration", data_root=str(root / "unused"), kill_at=checkpoint,
                       **_context(root))
    import tempfile
    with tempfile.TemporaryFile() as secret:
        descriptors = ()
        if request is not None and encrypted:
            secret.write(KEY.encode())
            secret.seek(0)
            request["fd"] = secret.fileno()
            descriptors = (secret.fileno(),)
        child = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, text=True, pass_fds=descriptors)
        try:
            if request is not None:
                child.stdin.write(json.dumps(request) + "\n")
                child.stdin.flush()
            assert select.select([child.stdout], [], [], 30)[0], "child did not reach publication boundary"
            line = child.stdout.readline().strip()
            if not line:
                child.wait(timeout=30)
                pytest.fail(child.stderr.read())
            assert json.loads(line) == {"checkpoint": checkpoint}, line
            child.kill()
            _, stderr = child.communicate(timeout=30)
            assert child.returncode < 0, stderr
        finally:
            if child.poll() is None:
                child.kill()
                child.communicate(timeout=30)


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child pipe harness")
@pytest.mark.parametrize("checkpoint", MOVE_POINTS + PUBLICATION_POINTS)
@pytest.mark.parametrize("starter,restarter", [(a, b) for a in ["python", "rust"] for b in ["python", "rust"]])
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_cross_runtime_resume_matches_uninterrupted_migration(storage_probe, tmp_path, encrypted, starter, restarter, checkpoint):
    _interrupted_case(storage_probe, tmp_path, encrypted, starter, restarter, checkpoint, "flat")


def _interrupted_case(binary, tmp_path, encrypted, starter, restarter, checkpoint, layout):
    root, seed = tmp_path / "case", tmp_path / "seed"
    _seed(root, encrypted, layout, 2)
    shutil.copytree(root, seed, copy_function=shutil.copy2)
    _finish(root, encrypted)
    expected_tree, expected_dumps = _logical_tree(_tree(root)), _databases(root, encrypted)
    shutil.rmtree(root)
    shutil.copytree(seed, root, copy_function=shutil.copy2)
    _kill_at(binary, starter, root, encrypted, checkpoint)
    if restarter == "python":
        _finish(root, encrypted)
    else:
        _finish_rust(binary, root, encrypted)
    assert _databases(root, encrypted) == expected_dumps
    assert _logical_tree(_tree(root)) == expected_tree
    marker = json.loads((root / "state/kassiber/projects/default/config/legacy-project-migration.json").read_bytes())
    assert "pending_legacy_artifacts" not in marker
    assert marker["legacy_artifacts_moved_at"] == STAMP
    _finish(root, encrypted)
    _finish_rust(binary, root, encrypted)
    assert _databases(root, encrypted) == expected_dumps
    assert _logical_tree(_tree(root)) == expected_tree


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child pipe harness")
@pytest.mark.parametrize("checkpoint", ["legacy.unlock_report", "legacy.validated"])
@pytest.mark.parametrize("starter,restarter", [(a, b) for a in ["python", "rust"] for b in ["python", "rust"]])
def test_deferred_sqlcipher_validation_resumes(storage_probe, tmp_path, starter, restarter, checkpoint):
    _interrupted_case(storage_probe, tmp_path, True, starter, restarter, checkpoint, "flat")


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child pipe harness")
@pytest.mark.parametrize("layout", ["xdg", "satbooks", "stale", "promoted"])
@pytest.mark.parametrize("starter,restarter", [("python", "rust"), ("rust", "python")])
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_other_layout_cleanup_cross_runtime_resume(storage_probe, tmp_path, layout, starter, restarter, encrypted):
    _interrupted_case(storage_probe, tmp_path, encrypted, starter, restarter, "legacy.after_move.database", layout)


@pytest.mark.parametrize("case", ["explicit-state-root", "populated-catalog", "backup-collision", "missing-artifacts"])
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
def test_selection_and_optional_artifacts_match_python(storage_probe, tmp_path, encrypted, case):
    root, seed = tmp_path / "case", tmp_path / "seed"
    _seed(root, encrypted, "flat", 2)
    state = root / "state/kassiber"
    options = {}
    if case == "explicit-state-root":
        options["state_root"] = str(state)
    elif case == "populated-catalog":
        with _reference_paths(root):
            projects.create_project("Other", project_id="other", encrypted=encrypted)
    elif case == "backup-collision":
        (state / "pre-project-migration-20261027T123456Z").mkdir(mode=0o700)
    elif case == "missing-artifacts":
        shutil.rmtree(state / "attachments")
        shutil.rmtree(state / "exports")
        (state / "config/backends.env").unlink()
        (state / "config/settings.json").unlink()
    shutil.copytree(root, seed, copy_function=shutil.copy2)
    with _reference_paths(root):
        entry = projects.ensure_default_project(**options)
    expected_tree, expected_dumps = _logical_tree(_tree(root)), _databases(root, encrypted)
    shutil.rmtree(root)
    shutil.copytree(seed, root, copy_function=shutil.copy2)
    result = _finish_rust(storage_probe, root, encrypted, ensure_only=True, **options)
    assert result == entry.to_catalog_dict()
    assert _databases(root, encrypted) == expected_dumps
    assert _logical_tree(_tree(root)) == expected_tree


@pytest.mark.skipif(os.name == "nt", reason="POSIX killed-child pipe harness")
@pytest.mark.parametrize("case", ["invalid-marker", "invalid-source", "wrong-key", "missing-key", "key-on-plaintext"])
def test_migration_refusals_preserve_python_error_codes(storage_probe, tmp_path, case):
    from kassiber.errors import AppError
    encrypted = case != "key-on-plaintext"
    root, seed = tmp_path / "case", tmp_path / "seed"
    _seed(root, encrypted, "flat", 2)
    _kill_at(storage_probe, "python", root, encrypted, "legacy.intent")
    marker_path = root / "state/kassiber/projects/default/config/legacy-project-migration.json"
    if case == "invalid-marker":
        marker_path.write_text("{", encoding="utf-8")
    elif case == "invalid-source":
        marker = json.loads(marker_path.read_bytes())
        marker["pending_legacy_artifacts"]["sources"]["database"] = str(root / "unrelated")
        marker_path.write_text(json.dumps(marker), encoding="utf-8")
    shutil.copytree(root, seed, copy_function=shutil.copy2)
    supplied = None if case == "missing-key" else ("wrong synthetic key" if case == "wrong-key" else KEY)
    with _reference_paths(root), pytest.raises(AppError) as error:
        entry = projects.ensure_default_project()
        connection = db.open_db(entry.data_root, passphrase=supplied)
        connection.close()
    expected_tree, expected_dumps = _logical_tree(_tree(root)), _databases(root, encrypted)
    shutil.rmtree(root)
    shutil.copytree(seed, root, copy_function=shutil.copy2)
    result = invoke(storage_probe, root / "unused", op="project-migration",
                    encrypted=case != "missing-key", passphrase=supplied or KEY, **_context(root))
    assert not result["ok"], result
    assert result["error"]["code"] == error.value.code
    assert result["error"]["retryable"] == error.value.retryable
    assert _databases(root, encrypted) == expected_dumps
    assert _logical_tree(_tree(root)) == expected_tree
