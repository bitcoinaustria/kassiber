from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal, Slot


class TransactionsViewModel(QObject):
    snapshotChanged = Signal()
    selectionChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot
        items = self._items()
        self._selected_id = items[0]["id"] if items else ""

    def _transactions(self) -> dict[str, Any]:
        return self._snapshot.get("transactions") or {}

    def _items(self) -> list[dict[str, Any]]:
        return list(self._transactions().get("items") or [])

    def _selected(self) -> dict[str, Any]:
        items = self._items()
        if not items:
            return {}
        for item in items:
            if item["id"] == self._selected_id:
                return item
        self._selected_id = items[0]["id"]
        return items[0]

    @Property(bool, notify=snapshotChanged)
    def isEmpty(self) -> bool:
        return len(self._items()) == 0

    @Property(int, notify=snapshotChanged)
    def totalCount(self) -> int:
        return int(self._transactions().get("total_count") or 0)

    @Property(str, notify=snapshotChanged)
    def historyLabel(self) -> str:
        return str(self._transactions().get("history_label") or "LOCAL SNAPSHOT")

    @Property("QVariantList", notify=snapshotChanged)
    def filterOptions(self):
        return list(self._transactions().get("filter_options") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def items(self):
        return self._items()

    @Property("QVariantMap", notify=selectionChanged)
    def selectedItem(self):
        return dict(self._selected())

    @Property("QVariantList", notify=selectionChanged)
    def selectedDetails(self):
        return list(self._selected().get("detail_rows") or [])

    @Slot(str)
    def selectTransaction(self, transaction_id: str) -> None:
        normalized = str(transaction_id or "").strip()
        if not normalized or normalized == self._selected_id:
            return
        for item in self._items():
            if item["id"] == normalized:
                self._selected_id = normalized
                self.selectionChanged.emit()
                return
