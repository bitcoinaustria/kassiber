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


def test_failed_hidden_home_move_raises_and_leaves_legacy_root_in_place(
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

    try:
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )
    except db.AppError as error:
        assert error.code == "state_root_migration_failed"
    else:
        raise AssertionError("migration unexpectedly succeeded")
    assert catalog.is_file()
    assert not (home / ".local" / "share" / "kassiber").exists()


def test_interrupted_publish_keeps_legacy_selected_and_resumes(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_rename = Path.rename

    def publish_then_fail(self, target):
        result = original_rename(self, target)
        if target == native and "state-migration-" in self.name:
            raise OSError("simulated interruption after publish")
        return result

    monkeypatch.setattr(Path, "rename", publish_then_fail)

    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    marker = native.parent / ".kassiber-state-migration.json"
    assert marker.is_file()
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert (native / "config" / "projects.json").is_file()
    monkeypatch.setattr(Path, "rename", original_rename)

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert not marker.exists()
    assert not (legacy / "config").exists()


def test_changed_source_after_publish_is_retained_and_migration_restarts(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_rename = Path.rename

    def publish_then_fail(self, target):
        result = original_rename(self, target)
        if target == native and "state-migration-" in self.name:
            raise OSError("simulated interruption after publish")
        return result

    monkeypatch.setattr(Path, "rename", publish_then_fail)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    changed = legacy / "config" / "changed-after-copy"
    changed.write_text("new state\n", encoding="utf-8")
    monkeypatch.setattr(Path, "rename", original_rename)
    with pytest.raises(db.AppError) as raised:
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    marker = native.parent / ".kassiber-state-migration.json"
    assert raised.value.code == "state_root_migration_source_changed"
    assert changed.read_text(encoding="utf-8") == "new state\n"
    assert not native.exists()
    assert not marker.exists()
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy

    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert (native / "config" / "changed-after-copy").is_file()


def test_source_to_backup_cutover_failure_keeps_full_legacy_authoritative(
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
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    assert catalog.is_file()
    assert database.read_text(encoding="utf-8") == "database state\n"
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy
    assert native.is_dir()
    assert (native.parent / ".kassiber-state-migration.json").is_file()


def test_cleanup_marker_resumes_after_backup_was_removed(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_clear = db._clear_migration_marker

    def interrupt_marker_clear(_path):
        raise OSError("simulated crash before marker clear")

    monkeypatch.setattr(db, "_clear_migration_marker", interrupt_marker_clear)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    marker = native.parent / ".kassiber-state-migration.json"
    payload = json.loads(marker.read_text(encoding="utf-8"))
    assert payload["phase"] == "finalizing"
    assert not Path(payload["backup"]).exists()
    assert native.is_dir()
    assert not (legacy / "config").exists()

    monkeypatch.setattr(db, "_clear_migration_marker", original_clear)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert not marker.exists()
    assert native.is_dir()


def test_incomplete_staging_copy_is_removed_and_rebuilt(monkeypatch, tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_copytree = db.shutil.copytree

    def interrupt_copy(_source, target, *args, **kwargs):
        (Path(target) / "incomplete").write_text("partial\n", encoding="utf-8")
        raise OSError("simulated interrupted copy")

    monkeypatch.setattr(db.shutil, "copytree", interrupt_copy)
    with pytest.raises(db.AppError):
        db.migrate_hidden_home_state_root_if_needed(
            platform="linux", environ={}, home=home
        )

    marker = native.parent / ".kassiber-state-migration.json"
    staging = Path(json.loads(marker.read_text(encoding="utf-8"))["staging"])
    assert (staging / "incomplete").is_file()
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy

    monkeypatch.setattr(db.shutil, "copytree", original_copytree)
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == native
    assert not staging.exists()
    assert not marker.exists()


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


def test_native_install_wins_when_native_and_hidden_home_both_have_state(
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
        home / ".local" / "share" / "kassiber"
    )
    assert db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    ) == (home / ".local" / "share" / "kassiber")


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
    assert not (native / "projects" / "default" / "data" / db.DEFAULT_DB_FILENAME).exists()


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
        with pytest.raises(db.AppError) as raised:
            db.migrate_hidden_home_state_root_if_needed(
                platform="linux", environ={}, home=home
            )

    assert raised.value.code == "state_root_migration_in_use"
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


def test_existing_legacy_fixed_path_wins_over_staged_duplicate(tmp_path: Path):
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


def test_migration_copies_to_target_filesystem_before_source_side_cutover(
    monkeypatch, tmp_path: Path
):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    original_rename = Path.rename
    original_copytree = db.shutil.copytree
    copied_legacy = False

    def reject_direct_cross_filesystem_rename(self, target):
        if self == legacy:
            assert target.parent == legacy.parent
            assert "state-migration-" in target.name
        return original_rename(self, target)

    def observe_copytree(source, target, *args, **kwargs):
        nonlocal copied_legacy
        copied_legacy = copied_legacy or Path(source) == legacy
        return original_copytree(source, target, *args, **kwargs)

    monkeypatch.setattr(Path, "rename", reject_direct_cross_filesystem_rename)
    monkeypatch.setattr(db.shutil, "copytree", observe_copytree)

    db.migrate_hidden_home_state_root_if_needed(
        platform="linux", environ={}, home=home
    )

    assert copied_legacy


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
