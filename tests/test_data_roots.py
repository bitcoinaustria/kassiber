import errno
import json
import os
from pathlib import Path

import pytest

from kassiber import db, update_check


def test_linux_native_state_root_honors_only_absolute_xdg_data_home(tmp_path: Path):
    home = tmp_path / "home"
    absolute = tmp_path / "xdg-data"

    assert db.native_state_root(
        platform="linux", environ={"XDG_DATA_HOME": str(absolute)}, home=home
    ) == absolute / "kassiber"
    for invalid in ("", "relative/data", "~/data"):
        assert db.native_state_root(
            platform="linux", environ={"XDG_DATA_HOME": invalid}, home=home
        ) == home / ".local" / "share" / "kassiber"


def test_native_state_root_uses_macos_and_windows_conventions(tmp_path: Path):
    home = tmp_path / "home"
    local_app_data = tmp_path / "local-app-data"
    identifier = "at.bitcoinaustria.kassiber"

    assert db.native_state_root(platform="darwin", environ={}, home=home) == (
        home / "Library" / "Application Support" / identifier
    )
    assert db.native_state_root(
        platform="win32",
        environ={"LOCALAPPDATA": str(local_app_data)},
        home=home,
    ) == local_app_data / identifier
    assert db.native_state_root(
        platform="win32", environ={"LOCALAPPDATA": "relative"}, home=home
    ) == home / "AppData" / "Local" / identifier


def test_existing_hidden_home_install_moves_but_bin_only_does_not(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"

    (legacy / "bin").mkdir(parents=True)
    (legacy / "run").mkdir()
    assert db.default_state_root(platform="linux", environ={}, home=home) == native

    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert (legacy / "bin").is_dir()
    assert (legacy / "run").is_dir()
    assert not (native / "bin").exists()
    assert not (native / "run").exists()
    assert not (legacy / "config").exists()
    assert (native / "config" / "projects.json").read_text(encoding="utf-8") == "{}\n"
    assert json.loads(
        (native / db.STATE_ROOT_MIGRATION_FILENAME).read_text(
            encoding="utf-8"
        )
    ) == {"schema_version": 1, "migrated_from_hidden_home": True}


def test_existing_hidden_home_database_moves_to_native_root(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    database = legacy / "projects" / "family" / "data" / db.DEFAULT_DB_FILENAME
    database.parent.mkdir(parents=True)
    database.touch()

    native = home / ".local" / "share" / "kassiber"
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert not (legacy / "projects").exists()
    assert (legacy / "run" / "operator-v1.start.lock").is_file()
    assert (native / "projects" / "family" / "data" / db.DEFAULT_DB_FILENAME).is_file()


def test_existing_native_root_is_never_merged_or_overwritten(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    legacy_catalog = home / ".kassiber" / "config" / "projects.json"
    native = home / ".local" / "share" / "kassiber"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")
    native.mkdir(parents=True)

    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert legacy_catalog.is_file()
    assert not (native / "config").exists()


def test_failed_hidden_home_move_leaves_legacy_root_in_place(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")

    def fail_rename(_self, _target):
        raise OSError("simulated move failure")

    monkeypatch.setattr(Path, "rename", fail_rename)

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert catalog.is_file()
    assert not (home / ".local" / "share" / "kassiber").exists()


def test_unwritable_native_parent_keeps_legacy_usable(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native_parent = home / ".local" / "share"
    state = legacy / "state"
    state.parent.mkdir(parents=True)
    state.write_text("state\n", encoding="utf-8")
    original_mkdir = Path.mkdir

    def reject_native_parent(path, *args, **kwargs):
        if path == native_parent:
            raise PermissionError("simulated read-only native parent")
        return original_mkdir(path, *args, **kwargs)

    monkeypatch.setattr(Path, "mkdir", reject_native_parent)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert state.is_file()


def test_interrupted_post_rename_finalization_resumes(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_restore = db._restore_fixed_state_paths

    def fail_after_rename(_moved_root, _legacy):
        raise OSError("simulated interruption after rename")

    monkeypatch.setattr(db, "_restore_fixed_state_paths", fail_after_rename)

    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    assert db._migration_proof_matches(native)
    assert db.default_state_root(platform="linux", environ={}, home=home) == native
    assert (native / "config" / "projects.json").is_file()
    monkeypatch.setattr(db, "_restore_fixed_state_paths", original_restore)

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert not (legacy / "config").exists()


def test_missing_prepared_proof_is_recreated_on_resume(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    state = legacy / "state"
    state.parent.mkdir(parents=True)
    state.write_text("state\n", encoding="utf-8")
    completion = legacy / db.STATE_ROOT_MIGRATION_FILENAME
    original_write = db._write_migration_json
    failed = False

    def fail_first_internal_proof(path, payload, **kwargs):
        nonlocal failed
        if path == completion and not failed:
            failed = True
            raise OSError("simulated proof write failure")
        return original_write(path, payload, **kwargs)

    monkeypatch.setattr(db, "_write_migration_json", fail_first_internal_proof)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert not completion.exists()
    assert state.is_file()

    monkeypatch.setattr(db, "_write_migration_json", original_write)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert (native / "state").is_file()


def test_atomic_migration_preserves_database_inode(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    database = legacy / "projects" / "default" / "data" / db.DEFAULT_DB_FILENAME
    database.parent.mkdir(parents=True)
    database.write_text("state\n", encoding="utf-8")
    inode = database.stat().st_ino

    db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    )

    moved_database = (
        native / "projects" / "default" / "data" / db.DEFAULT_DB_FILENAME
    )
    assert moved_database.stat().st_ino == inode


def test_atomic_cutover_failure_keeps_full_legacy_authoritative(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    database = legacy / "projects" / "default" / "data" / db.DEFAULT_DB_FILENAME
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    database.parent.mkdir(parents=True)
    database.write_text("database state\n", encoding="utf-8")
    original_rename = Path.rename

    def fail_windows_style_cutover(self, target):
        if self == legacy:
            raise PermissionError("simulated open Windows handle")
        return original_rename(self, target)

    monkeypatch.setattr(Path, "rename", fail_windows_style_cutover)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy

    assert catalog.is_file()
    assert database.read_text(encoding="utf-8") == "database state\n"
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert not native.exists()
    assert db._migration_proof_matches(legacy)


def test_cross_filesystem_move_keeps_legacy_untouched(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_stat = Path.stat

    def other_device(path, *args, **kwargs):
        result = original_stat(path, *args, **kwargs)
        if path == native.parent:
            values = list(result)
            values[2] = result.st_dev + 1
            return os.stat_result(values)
        return result

    monkeypatch.setattr(Path, "stat", other_device)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert catalog.is_file()
    assert not native.exists()


def test_rename_exdev_clears_journal_and_keeps_legacy(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    state = legacy / "state"
    state.parent.mkdir(parents=True)
    state.write_text("state\n", encoding="utf-8")
    original_rename = Path.rename

    def fail_cross_device(path, target):
        if path == legacy and target == native:
            raise OSError(errno.EXDEV, "cross-device")
        return original_rename(path, target)

    monkeypatch.setattr(Path, "rename", fail_cross_device)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert state.is_file()
    assert not (legacy / db.STATE_ROOT_MIGRATION_FILENAME).exists()


def test_dangling_native_symlink_is_not_overwritten(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    legacy_catalog = home / ".kassiber" / "config" / "projects.json"
    native = home / ".local" / "share" / "kassiber"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")
    native.parent.mkdir(parents=True)
    native.symlink_to(tmp_path / "missing-native-root")

    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert native.is_symlink()
    assert legacy_catalog.is_file()


def test_legacy_install_wins_when_both_roots_have_state(
    tmp_path: Path,
):
    home = tmp_path / "home"
    legacy_catalog = home / ".kassiber" / "config" / "projects.json"
    native_catalog = (
        home / ".local" / "share" / "kassiber" / "config" / "projects.json"
    )
    for catalog in (legacy_catalog, native_catalog):
        catalog.parent.mkdir(parents=True)
        catalog.write_text("{}\n", encoding="utf-8")

    assert db.default_state_root(platform="linux", environ={}, home=home) == (
        home / ".kassiber"
    )
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == home / ".kassiber"


def test_hidden_home_database_beats_native_catalog_only(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    legacy_database = (
        home / ".kassiber" / "projects" / "default" / "data" / db.DEFAULT_DB_FILENAME
    )
    native_catalog = (
        home / ".local" / "share" / "kassiber" / "config" / "projects.json"
    )
    legacy_database.parent.mkdir(parents=True)
    legacy_database.touch()
    native_catalog.parent.mkdir(parents=True)
    native_catalog.write_text("{}\n", encoding="utf-8")

    native = home / ".local" / "share" / "kassiber"
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert legacy_database.is_file()
    assert not (
        native / "projects" / "default" / "data" / db.DEFAULT_DB_FILENAME
    ).exists()


@pytest.mark.skipif(os.name == "nt", reason="Unix advisory-lock behavior")
@pytest.mark.parametrize(
    "relative_lock",
    (
        Path("projects/default/.operator-owner.lock"),
        Path("run/operator-v1.start.lock"),
    ),
)
def test_active_legacy_operator_lock_blocks_hidden_home_move(
    tmp_path: Path, relative_lock: Path
):
    import fcntl

    home = tmp_path / "home"
    legacy = home / ".kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    lock_path = legacy / relative_lock
    lock_path.parent.mkdir(parents=True)
    with lock_path.open("a+b") as lock_file:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        ) == legacy

    assert catalog.is_file()


def test_windows_active_legacy_owner_lock_blocks_hidden_home_move(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    lock_path = legacy / "projects" / "default" / ".operator-owner.lock"
    lock_path.parent.mkdir(parents=True)
    lock_path.write_text("", encoding="utf-8")

    monkeypatch.setattr(db.os, "name", "nt")
    monkeypatch.setattr(
        db,
        "_legacy_owner_lock_is_active",
        lambda candidate: candidate == lock_path,
    )

    with pytest.raises(db.AppError) as raised:
        with db._hold_legacy_operator_locks(legacy):
            pass

    assert raised.value.code == "state_root_migration_in_use"


def test_independent_native_fixed_paths_are_not_touched_without_marker(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = native / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    (native / "bin").mkdir()
    (native / "run").mkdir()

    assert (
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )
        == native
    )
    assert not legacy.exists()
    assert (native / "bin").is_dir()
    assert (native / "run").is_dir()


def test_existing_legacy_fixed_path_is_restored_after_move(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    legacy_catalog = legacy / "config" / "projects.json"
    legacy_catalog.parent.mkdir(parents=True)
    legacy_catalog.write_text("{}\n", encoding="utf-8")
    (legacy / "bin").mkdir(parents=True)
    launcher = legacy / "bin" / "kassiber"
    launcher.write_text("launcher\n", encoding="utf-8")

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native

    assert launcher.read_text(encoding="utf-8") == "launcher\n"
    assert not (native / "bin").exists()
    assert (legacy.stat().st_mode & 0o777) == 0o700


@pytest.mark.skipif(os.name == "nt", reason="symlink behavior is platform-specific")
def test_fixed_bin_symlink_does_not_block_state_move(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    (legacy / "config").mkdir(parents=True)
    (legacy / "config" / "projects.json").write_text("{}\n", encoding="utf-8")
    target = tmp_path / "launcher"
    target.write_text("launcher\n", encoding="utf-8")
    (legacy / "bin").symlink_to(target)

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert (legacy / "bin").is_symlink()
    assert not (native / "bin").exists()


def test_any_non_fixed_top_level_entry_is_meaningful(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    legacy.mkdir(parents=True)
    (legacy / "custom-state").write_text("keep me\n", encoding="utf-8")

    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert (native / "custom-state").read_text(encoding="utf-8") == "keep me\n"


@pytest.mark.parametrize("at_root", [True, False])
def test_legacy_symlinks_retain_legacy_root(tmp_path: Path, at_root: bool):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    outside = tmp_path / "outside"
    outside.mkdir()
    if at_root:
        legacy.parent.mkdir(parents=True)
        legacy.symlink_to(outside, target_is_directory=True)
        (outside / "state").write_text("outside\n", encoding="utf-8")
    else:
        legacy.mkdir(parents=True)
        (legacy / "state").symlink_to(outside, target_is_directory=True)

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == legacy
    assert legacy.is_symlink() if at_root else (legacy / "state").is_symlink()


def test_fixed_path_conflict_after_rename_fails_without_deleting_either(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    (legacy / "bin").mkdir(parents=True)
    (legacy / "bin" / "original").write_text("original\n", encoding="utf-8")
    (legacy / "state").write_text("state\n", encoding="utf-8")
    original_restore = db._restore_fixed_state_paths

    def interrupt_restore(_moved_root, _legacy):
        raise db.AppError("interrupted", code="state_root_migration_failed")

    monkeypatch.setattr(db, "_restore_fixed_state_paths", interrupt_restore)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )
    monkeypatch.setattr(db, "_restore_fixed_state_paths", original_restore)
    (legacy / "bin").mkdir(parents=True)
    (legacy / "bin" / "new").write_text("new\n", encoding="utf-8")

    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    assert (native / "bin" / "original").is_file()
    assert (legacy / "bin" / "new").is_file()


@pytest.mark.skipif(os.name == "nt", reason="symlink behavior is platform-specific")
def test_legacy_symlink_after_cutover_is_not_followed(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    (legacy / "bin").mkdir(parents=True)
    (legacy / "bin" / "original").write_text("original\n", encoding="utf-8")
    (legacy / "state").write_text("state\n", encoding="utf-8")
    original_restore = db._restore_fixed_state_paths

    def interrupt_restore(_moved_root, _legacy):
        raise db.AppError("interrupted", code="state_root_migration_failed")

    monkeypatch.setattr(db, "_restore_fixed_state_paths", interrupt_restore)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )
    monkeypatch.setattr(db, "_restore_fixed_state_paths", original_restore)

    outside = tmp_path / "outside"
    outside.mkdir()
    legacy.symlink_to(outside, target_is_directory=True)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    assert not any(outside.iterdir())
    assert (native / "bin" / "original").is_file()


def test_new_legacy_state_after_cutover_blocks_completion(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    (legacy / "state").mkdir(parents=True)
    original_restore = db._restore_fixed_state_paths

    def interrupt_restore(_moved_root, _legacy):
        raise db.AppError("interrupted", code="state_root_migration_failed")

    monkeypatch.setattr(db, "_restore_fixed_state_paths", interrupt_restore)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )
    monkeypatch.setattr(db, "_restore_fixed_state_paths", original_restore)
    (legacy / "new-state").mkdir(parents=True)

    with pytest.raises(db.AppError) as raised:
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    assert raised.value.code == "state_root_conflict"
    assert (native / "state").is_dir()
    assert (legacy / "new-state").is_dir()


@pytest.mark.skipif(not hasattr(os, "mkfifo"), reason="FIFO unavailable")
def test_atomic_rename_preserves_special_files(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    fifo = legacy / "custom.fifo"
    fifo.parent.mkdir(parents=True)
    os.mkfifo(fifo)

    native = db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    )

    assert (native / "custom.fifo").exists()


def test_project_catalog_paths_below_legacy_root_are_rebased(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    external = tmp_path / "external-project"
    normalized_external = home / "external"
    external_via_legacy = legacy / "projects" / "old" / ".." / ".." / ".." / "external"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text(
        json.dumps(
            {
                "projects": [
                    {"id": "local", "path": str(legacy / "projects" / "local")},
                    {"id": "external", "path": str(external)},
                    {"id": "normalized-external", "path": str(external_via_legacy)},
                ]
            }
        ),
        encoding="utf-8",
    )

    db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    )

    migrated = json.loads(
        (native / "config" / "projects.json").read_text(encoding="utf-8")
    )
    assert migrated["projects"][0]["path"] == str(native / "projects" / "local")
    assert migrated["projects"][1]["path"] == str(external)
    assert os.path.normpath(migrated["projects"][2]["path"]) == str(
        normalized_external
    )
    assert migrated["projects"][2]["path"] == str(external_via_legacy)


def test_native_flat_database_is_used_when_legacy_has_only_fixed_state(tmp_path: Path):
    home = tmp_path / "home"
    native = home / ".local" / "share" / "kassiber"
    database = native / db.DEFAULT_DB_FILENAME
    database.parent.mkdir(parents=True)
    database.touch()
    (home / ".kassiber" / "bin").mkdir(parents=True)
    (home / ".kassiber" / "run").mkdir()

    assert db.default_state_root(platform="linux", environ={}, home=home) == native


def test_update_preferences_live_below_the_effective_global_state_root(
    monkeypatch, tmp_path: Path
):
    monkeypatch.setattr(update_check, "default_state_root", lambda: tmp_path)

    assert update_check.preference_path() == (
        tmp_path / db.DEFAULT_CONFIG_DIRNAME / update_check.PREFERENCE_FILENAME
    )
    assert update_check.cache_path() == (
        tmp_path / db.DEFAULT_CONFIG_DIRNAME / update_check.CACHE_FILENAME
    )
