from pathlib import Path

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


def test_existing_hidden_home_install_wins_but_bin_only_does_not(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    native = home / ".local" / "share" / "kassiber"

    (legacy / "bin").mkdir(parents=True)
    assert db.default_state_root(platform="linux", environ={}, home=home) == native

    catalog = legacy / "config" / "projects.json"
    catalog.parent.mkdir(parents=True)
    catalog.write_text("{}\n", encoding="utf-8")
    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy


def test_existing_hidden_home_database_is_not_stranded(tmp_path: Path):
    home = tmp_path / "home"
    legacy = home / ".kassiber"
    database = legacy / "projects" / "family" / "data" / db.DEFAULT_DB_FILENAME
    database.parent.mkdir(parents=True)
    database.touch()

    assert db.default_state_root(platform="linux", environ={}, home=home) == legacy


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


def test_hidden_home_database_beats_native_catalog_only(tmp_path: Path):
    home = tmp_path / "home"
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

    assert db.default_state_root(platform="linux", environ={}, home=home) == (
        home / ".kassiber"
    )


def test_update_preferences_live_below_the_effective_global_state_root(
    monkeypatch, tmp_path: Path
):
    monkeypatch.setattr(update_check, "DEFAULT_STATE_ROOT", str(tmp_path))

    assert update_check.preference_path() == (
        tmp_path / db.DEFAULT_CONFIG_DIRNAME / update_check.PREFERENCE_FILENAME
    )
    assert update_check.cache_path() == (
        tmp_path / db.DEFAULT_CONFIG_DIRNAME / update_check.CACHE_FILENAME
    )
