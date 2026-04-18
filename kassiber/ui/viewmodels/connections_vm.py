from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal


class ConnectionsViewModel(QObject):
    snapshotChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot

    def _shell(self) -> dict[str, Any]:
        return self._snapshot.get("shell") or {}

    def _scope(self) -> dict[str, Any]:
        return self._snapshot.get("scope") or {}

    @Property(bool, notify=snapshotChanged)
    def isEmpty(self) -> bool:
        return bool(self._shell().get("is_empty", True))

    @Property(int, notify=snapshotChanged)
    def connectionCount(self) -> int:
        return int(self._shell().get("connection_count", 0))

    @Property(str, notify=snapshotChanged)
    def ctaLabel(self) -> str:
        return "+ Add Connection"

    @Property(bool, notify=snapshotChanged)
    def canOpenAddConnection(self) -> bool:
        return bool(self._scope())

    @Property(str, notify=snapshotChanged)
    def emptyBadge(self) -> str:
        return "No Connections" if self.isEmpty else "Connections Found"
