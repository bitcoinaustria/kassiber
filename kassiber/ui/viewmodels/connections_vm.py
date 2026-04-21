from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal, Slot


class ConnectionsViewModel(QObject):
    snapshotChanged = Signal()
    selectionChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot
        items = self._items()
        self._selected_id = items[0]["id"] if items else ""

    def _scope(self) -> dict[str, Any]:
        return self._snapshot.get("scope") or {}

    def _connections(self) -> dict[str, Any]:
        return self._snapshot.get("connections") or {}

    def _items(self) -> list[dict[str, Any]]:
        return list(self._connections().get("items") or [])

    def _selected(self) -> dict[str, Any]:
        items = self._items()
        if not items:
            return {}
        for item in items:
            if item["id"] == self._selected_id:
                return item
        self._selected_id = items[0]["id"]
        return items[0]

    def _transactions(self) -> list[dict[str, Any]]:
        return list((self._snapshot.get("transactions") or {}).get("items") or [])

    def _selected_transactions(self) -> list[dict[str, Any]]:
        selected = self._selected()
        if not selected:
            return []
        label = str(selected.get("label") or "").strip().lower()
        if not label:
            return self._transactions()[:6]
        matches = [item for item in self._transactions() if label in str(item.get("wallet") or "").lower()]
        return matches[:6] if matches else self._transactions()[:6]

    @Property(bool, notify=snapshotChanged)
    def isEmpty(self) -> bool:
        return len(self._items()) == 0

    @Property(int, notify=snapshotChanged)
    def connectionCount(self) -> int:
        return len(self._items())

    @Property(str, notify=snapshotChanged)
    def ctaLabel(self) -> str:
        return "+ Add Connection"

    @Property(bool, notify=snapshotChanged)
    def canOpenAddConnection(self) -> bool:
        return bool(self._scope())

    @Property(str, notify=snapshotChanged)
    def emptyBadge(self) -> str:
        return "No Connections" if self.isEmpty else "Connections Found"

    @Property("QVariantList", notify=snapshotChanged)
    def items(self):
        return self._items()

    @Property(str, notify=selectionChanged)
    def selectedId(self) -> str:
        return self._selected().get("id", "")

    @Property("QVariantMap", notify=selectionChanged)
    def selectedItem(self):
        return dict(self._selected())

    @Property("QVariantList", notify=selectionChanged)
    def selectedDetails(self):
        return list(self._selected().get("detail_rows") or [])

    @Property("QVariantList", notify=selectionChanged)
    def selectedTransactions(self):
        return self._selected_transactions()

    @Slot(str)
    def selectConnection(self, connection_id: str) -> None:
        normalized = str(connection_id or "").strip()
        if not normalized or normalized == self._selected_id:
            return
        for item in self._items():
            if item["id"] == normalized:
                self._selected_id = normalized
                self.selectionChanged.emit()
                return
