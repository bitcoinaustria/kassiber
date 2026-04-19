from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal


class SettingsViewModel(QObject):
    snapshotChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot

    def _status(self) -> dict[str, Any]:
        return self._snapshot.get("status") or {}

    def _settings(self) -> dict[str, Any]:
        return self._snapshot.get("settings") or {}

    @Property(str, notify=snapshotChanged)
    def versionText(self) -> str:
        version = self._status().get("version", "")
        return f"v{version}" if version else "v?"

    @Property(str, notify=snapshotChanged)
    def settingsFile(self) -> str:
        return self._status().get("settings_file", "")

    @Property(str, notify=snapshotChanged)
    def envFile(self) -> str:
        return self._status().get("env_file", "")

    @Property("QVariantList", notify=snapshotChanged)
    def cards(self):
        return list(self._settings().get("cards") or [])
