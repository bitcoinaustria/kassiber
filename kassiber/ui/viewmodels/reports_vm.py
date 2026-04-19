from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal


class ReportsViewModel(QObject):
    snapshotChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot

    def _reports(self) -> dict[str, Any]:
        return self._snapshot.get("reports") or {}

    @Property(str, notify=snapshotChanged)
    def statusTitle(self) -> str:
        return self._reports().get("status_title", "")

    @Property(str, notify=snapshotChanged)
    def statusBody(self) -> str:
        return self._reports().get("status_body", "")

    @Property(str, notify=snapshotChanged)
    def statusTone(self) -> str:
        return self._reports().get("status_tone", "warn")

    @Property("QVariantList", notify=snapshotChanged)
    def items(self):
        return list(self._reports().get("items") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def summaryCards(self):
        return list(self._reports().get("summary_cards") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def methodOptions(self):
        return list(self._reports().get("method_options") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def policyRows(self):
        return list(self._reports().get("policy_rows") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def previewRows(self):
        return list(self._reports().get("preview_rows") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def exportFormats(self):
        return list(self._reports().get("export_formats") or [])
