from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal, Slot


class DashboardViewModel(QObject):
    snapshotChanged = Signal()
    pageChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot
        self._page_order = [
            "welcome",
            "overview",
            "connection-detail",
            "transactions",
            "reports",
            "settings",
            "profiles",
        ]
        self._current_page = "overview" if self.hasProfile else "welcome"

    def _shell(self) -> dict[str, Any]:
        return self._snapshot.get("shell") or {}

    def _scope(self) -> dict[str, Any]:
        return self._snapshot.get("scope") or {}

    def _status(self) -> dict[str, Any]:
        return self._snapshot.get("status") or {}

    def _welcome(self) -> dict[str, Any]:
        return self._snapshot.get("welcome") or {}

    def _overview(self) -> dict[str, Any]:
        return self._snapshot.get("overview") or {}

    def _page_enabled(self, page_id: str) -> bool:
        if page_id in {"welcome", "settings"}:
            return True
        return self.hasProfile

    @Property("QVariantList", notify=snapshotChanged)
    def pages(self):
        return [
            {
                "id": "welcome",
                "label": "Welcome",
                "caption": "Landing and setup",
                "enabled": True,
            },
            {
                "id": "overview",
                "label": "Overview",
                "caption": "Empty and data states",
                "enabled": self._page_enabled("overview"),
            },
            {
                "id": "connection-detail",
                "label": "Connection Detail",
                "caption": "Wallets and config",
                "enabled": self._page_enabled("connection-detail"),
            },
            {
                "id": "transactions",
                "label": "Transaction View",
                "caption": "Recent activity",
                "enabled": self._page_enabled("transactions"),
            },
            {
                "id": "reports",
                "label": "Tax Reports",
                "caption": "Read-only report surface",
                "enabled": self._page_enabled("reports"),
            },
            {
                "id": "settings",
                "label": "Settings",
                "caption": "Paths and preferences",
                "enabled": True,
            },
        ]

    @Property(str, notify=pageChanged)
    def currentPage(self) -> str:
        return self._current_page

    @Property(int, notify=pageChanged)
    def pageIndex(self) -> int:
        try:
            return self._page_order.index(self._current_page)
        except ValueError:
            return 0

    @Slot(str)
    def selectPage(self, page_id: str) -> None:
        normalized = str(page_id or "").strip()
        if normalized not in self._page_order:
            return
        if not self._page_enabled(normalized):
            return
        if normalized == self._current_page:
            return
        self._current_page = normalized
        self.pageChanged.emit()

    @Property(str, notify=snapshotChanged)
    def windowTitle(self) -> str:
        return self._shell().get("window_title", "Kassiber")

    @Property(str, notify=snapshotChanged)
    def shellTitle(self) -> str:
        if not self.hasProfile:
            return self._shell().get("empty_state_title", "Create a profile in the CLI first")
        if self.isEmpty:
            return self._shell().get("empty_state_title", "Add a connection")
        return self._shell().get("placeholder_title", "Connections detected")

    @Property(str, notify=snapshotChanged)
    def shellBody(self) -> str:
        if not self.hasProfile:
            return self._shell().get("empty_state_body", "")
        if not self.hasData:
            return self._shell().get("empty_state_body", "")
        return self._shell().get("placeholder_body", "")

    @Property(bool, notify=snapshotChanged)
    def hasProfile(self) -> bool:
        return bool(self._scope())

    @Property(bool, notify=snapshotChanged)
    def isEmpty(self) -> bool:
        return bool(self._shell().get("is_empty", True))

    @Property(bool, notify=snapshotChanged)
    def hasData(self) -> bool:
        return bool(self._shell().get("has_data", False))

    @Property(str, notify=snapshotChanged)
    def projectLabel(self) -> str:
        return self._shell().get("project_label", "No project selected")

    @Property(str, notify=snapshotChanged)
    def projectSummary(self) -> str:
        if not self.hasProfile:
            return "Desktop shell"
        return (
            f"{self._shell().get('current_workspace_label', '')} / "
            f"{self._shell().get('current_profile_label', '')}"
        )

    @Property(str, notify=snapshotChanged)
    def currentWorkspaceLabel(self) -> str:
        if not self.hasProfile:
            return "Kassiber"
        return self._shell().get("current_workspace_label", "") or ""

    @Property(str, notify=snapshotChanged)
    def currentProfileLabel(self) -> str:
        if not self.hasProfile:
            return ""
        return self._shell().get("current_profile_label", "") or ""

    @Property("QVariantList", notify=snapshotChanged)
    def availableProfiles(self):
        return list(self._snapshot.get("profiles") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def notices(self):
        return list(self._shell().get("notices") or [])

    @Property(str, notify=snapshotChanged)
    def footerSummary(self) -> str:
        status = self._status()
        return f"Data root: {status.get('data_root', '')}"

    @Property(str, notify=snapshotChanged)
    def phaseSummary(self) -> str:
        return "UI scaffold: routed screens over live read-only snapshot data."

    @Property(str, notify=snapshotChanged)
    def welcomeTitle(self) -> str:
        return self._welcome().get("title", "")

    @Property(str, notify=snapshotChanged)
    def welcomeBody(self) -> str:
        return self._welcome().get("body", "")

    @Property(str, notify=snapshotChanged)
    def welcomeWorkspaceValue(self) -> str:
        return self._welcome().get("workspace_value", "")

    @Property(str, notify=snapshotChanged)
    def welcomeNamePlaceholder(self) -> str:
        return self._welcome().get("name_placeholder", "")

    @Property(str, notify=snapshotChanged)
    def welcomeResidencyNote(self) -> str:
        return self._welcome().get("residency_note", "")

    @Property(str, notify=snapshotChanged)
    def welcomeStampCaption(self) -> str:
        return self._welcome().get("stamp_caption", "")

    @Property("QVariantList", notify=snapshotChanged)
    def welcomeResidencyOptions(self):
        return list(self._welcome().get("residency_options") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def overviewMetrics(self):
        return list(self._overview().get("metrics") or [])

    @Property("QVariantList", notify=snapshotChanged)
    def overviewHighlights(self):
        return list(self._overview().get("highlights") or [])
