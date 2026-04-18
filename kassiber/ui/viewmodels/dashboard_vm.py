from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, Signal


class DashboardViewModel(QObject):
    snapshotChanged = Signal()

    def __init__(self, snapshot: dict[str, Any]) -> None:
        super().__init__()
        self._snapshot = snapshot

    def _shell(self) -> dict[str, Any]:
        return self._snapshot.get("shell") or {}

    def _scope(self) -> dict[str, Any]:
        return self._snapshot.get("scope") or {}

    def _status(self) -> dict[str, Any]:
        return self._snapshot.get("status") or {}

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
        if self.isEmpty:
            return self._shell().get("empty_state_body", "")
        return self._shell().get("placeholder_body", "")

    @Property(bool, notify=snapshotChanged)
    def hasProfile(self) -> bool:
        return bool(self._scope())

    @Property(bool, notify=snapshotChanged)
    def isEmpty(self) -> bool:
        return bool(self._shell().get("is_empty", True))

    @Property(str, notify=snapshotChanged)
    def projectLabel(self) -> str:
        return self._shell().get("project_label", "No project selected")

    @Property(str, notify=snapshotChanged)
    def projectSummary(self) -> str:
        if not self.hasProfile:
            return "Phase 1 app shell"
        return (
            f"{self._shell().get('current_workspace_label', '')} / "
            f"{self._shell().get('current_profile_label', '')}"
        )

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
        return "Phase 1 shell: QML frame, empty state, placeholder dialogs."
