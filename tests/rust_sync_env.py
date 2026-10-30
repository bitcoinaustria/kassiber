"""Disposable child state for Bitcoin sync oracles, preserving account HOME."""
import os


def child_env(tmp_path):
    # Child failure reports must never contain unrelated parent credentials.
    allowed = {"PATH", "LANG", "LC_ALL", "TZ", "HOME", "USERPROFILE", "LOCALAPPDATA", "TMPDIR",
               "XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "COLUMNS", "LINES",
               "PYTHONIOENCODING", "PYTHONPATH", "KASSIBER_TEST_NO_EGRESS",
               "KASSIBER_DISABLE_UPDATE_CHECK", "KASSIBER_OFFLINE_PREFERENCE_FILE",
               "KASSIBER_AGENT_ACCESS_FILE", "KASSIBER_UPDATE_PREFERENCE_FILE",
               "KASSIBER_UPDATE_CACHE_FILE"}
    env = {key: value for key, value in os.environ.items() if key in allowed}
    env.update({
        "TMPDIR": str(tmp_path),
        "USERPROFILE": str(tmp_path), "LOCALAPPDATA": str(tmp_path),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "XDG_CONFIG_HOME": str(tmp_path / "xdg-config"),
        "XDG_STATE_HOME": str(tmp_path / "xdg-state"),
        "COLUMNS": "80", "LINES": "24", "PYTHONIOENCODING": "utf-8",
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
        "KASSIBER_AGENT_ACCESS_FILE": str(tmp_path / "agents.json"),
        "KASSIBER_UPDATE_PREFERENCE_FILE": str(tmp_path / "updates.json"),
        "KASSIBER_UPDATE_CACHE_FILE": str(tmp_path / "update-cache.json"),
    })
    return env
