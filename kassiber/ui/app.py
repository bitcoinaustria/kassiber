from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from ..db import resolve_settings_path
from ..errors import AppError
from .dashboard import collect_ui_snapshot


def _import_qt():
    try:
        from PySide6.QtCore import QUrl
        from PySide6.QtGui import QFontDatabase, QGuiApplication
        from PySide6.QtQml import QQmlApplicationEngine
        from PySide6.QtQuickControls2 import QQuickStyle
    except Exception as exc:  # pragma: no cover - local Qt install dependent
        raise AppError(
            "kassiber ui requires the PySide6 runtime in the active Python environment.",
            code="ui_unavailable",
            hint="Reinstall Kassiber so the desktop dependency set is available, or use `kassiber --machine ui`.",
        ) from exc
    return QUrl, QFontDatabase, QGuiApplication, QQmlApplicationEngine, QQuickStyle


def _register_bundled_fonts(font_database) -> None:
    """Register every .ttf under resources/fonts/ with QFontDatabase.

    Silently skips missing or unreadable files; system-installed fonts still
    work as a fallback via the theme.py family names.
    """
    fonts_dir = Path(__file__).resolve().parent / "resources" / "fonts"
    if not fonts_dir.exists():
        return
    for ttf in fonts_dir.rglob("*.ttf"):
        try:
            font_database.addApplicationFont(str(ttf))
        except Exception:
            continue


def _default_window_state() -> dict[str, int]:
    return {"x": -1, "y": -1, "width": 1240, "height": 820}


def _load_settings_blob(settings_path: Path) -> dict[str, Any]:
    if not settings_path.exists():
        return {}
    try:
        payload = json.loads(settings_path.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError):
        return {}
    return payload if isinstance(payload, dict) else {}


def _read_window_state(settings_path: Path) -> dict[str, int]:
    state = dict(_default_window_state())
    ui_section = _load_settings_blob(settings_path).get("ui")
    if not isinstance(ui_section, dict):
        return state
    window = ui_section.get("window")
    if not isinstance(window, dict):
        return state
    for key in state:
        value = window.get(key)
        if isinstance(value, int):
            state[key] = value
    return state


def _write_window_state(settings_path: Path, window) -> None:
    payload = _load_settings_blob(settings_path)
    ui_section = dict(payload.get("ui")) if isinstance(payload.get("ui"), dict) else {}
    ui_section["window"] = {
        "x": int(window.property("x")),
        "y": int(window.property("y")),
        "width": int(window.property("width")),
        "height": int(window.property("height")),
    }
    payload["ui"] = ui_section
    try:
        settings_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    except OSError:
        # Best-effort persistence only; screenshot/offscreen flows may not have permission.
        return


def _qml_path() -> Path:
    return Path(__file__).resolve().parent / "resources" / "qml" / "Main.qml"


def _apply_preview_scene(snapshot: dict[str, Any], preview_scene: str) -> str:
    scene = str(preview_scene or "").strip().lower()
    if not scene:
        return ""

    page_map = {
        "welcome": "welcome",
        "overview": "overview",
        "overview-empty": "overview",
        "overview-data": "overview",
        "transactions": "transactions",
        "tax": "reports",
        "reports": "reports",
        "tax-capital-gains": "reports",
        "connection-detail": "connection-detail",
        "settings": "settings",
        "profiles": "profiles",
    }
    page = page_map.get(scene, scene)

    if scene == "overview-empty":
        shell = dict(snapshot.get("shell") or {})
        shell["is_empty"] = True
        shell["has_data"] = False
        shell["connection_count"] = 0
        snapshot["shell"] = shell
        snapshot["connections"] = {"items": []}
        snapshot["transactions"] = {"items": []}

    return page


def build_application(
    conn,
    data_root: str,
    runtime_config: dict[str, Any],
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
):
    QUrl, QFontDatabase, QGuiApplication, QQmlApplicationEngine, QQuickStyle = _import_qt()
    from .theme import Theme
    from .viewmodels.connections_vm import ConnectionsViewModel
    from .viewmodels.dashboard_vm import DashboardViewModel
    from .viewmodels.reports_vm import ReportsViewModel
    from .viewmodels.settings_vm import SettingsViewModel
    from .viewmodels.transactions_vm import TransactionsViewModel

    snapshot = collect_ui_snapshot(
        conn,
        data_root,
        runtime_config,
        workspace_ref=workspace_ref,
        profile_ref=profile_ref,
    )
    preview_scene = (os.environ.get("KASSIBER_UI_PREVIEW_PAGE") or "").strip()
    capture_mode = (os.environ.get("KASSIBER_UI_CAPTURE") or "").strip().lower() in {"1", "true", "yes", "on"}
    preview_page = _apply_preview_scene(snapshot, preview_scene)
    settings_path = resolve_settings_path(data_root)
    window_state = _read_window_state(settings_path)

    QQuickStyle.setStyle("Basic")
    app = QGuiApplication.instance() or QGuiApplication(["kassiber"])
    app.setApplicationName("Kassiber")
    app.setApplicationDisplayName("Kassiber")
    _register_bundled_fonts(QFontDatabase)

    dashboard_vm = DashboardViewModel(snapshot)
    if preview_page:
        dashboard_vm.selectPage(preview_page)
    connections_vm = ConnectionsViewModel(snapshot)
    transactions_vm = TransactionsViewModel(snapshot)
    reports_vm = ReportsViewModel(snapshot)
    settings_vm = SettingsViewModel(snapshot)
    theme = Theme()

    engine = QQmlApplicationEngine()
    dashboard_vm.setParent(engine)
    connections_vm.setParent(engine)
    transactions_vm.setParent(engine)
    reports_vm.setParent(engine)
    settings_vm.setParent(engine)
    theme.setParent(engine)
    context = engine.rootContext()
    context.setContextProperty("dashboardVM", dashboard_vm)
    context.setContextProperty("connectionsVM", connections_vm)
    context.setContextProperty("transactionsVM", transactions_vm)
    context.setContextProperty("reportsVM", reports_vm)
    context.setContextProperty("settingsVM", settings_vm)
    context.setContextProperty("theme", theme)
    context.setContextProperty("windowState", window_state)
    context.setContextProperty("uiPreviewPage", preview_scene)
    context.setContextProperty("uiCaptureMode", capture_mode)
    engine._kassiber_refs = {
        "dashboard_vm": dashboard_vm,
        "connections_vm": connections_vm,
        "transactions_vm": transactions_vm,
        "reports_vm": reports_vm,
        "settings_vm": settings_vm,
        "theme": theme,
    }
    engine.load(QUrl.fromLocalFile(str(_qml_path())))
    if not engine.rootObjects():
        raise AppError("Failed to load the Kassiber QML shell.", code="ui_unavailable")

    window = engine.rootObjects()[0]
    if window_state["x"] >= 0:
        window.setProperty("x", window_state["x"])
    if window_state["y"] >= 0:
        window.setProperty("y", window_state["y"])
    if window_state["width"] > 0:
        window.setProperty("width", window_state["width"])
    if window_state["height"] > 0:
        window.setProperty("height", window_state["height"])
    window.setProperty("visible", True)
    app.aboutToQuit.connect(lambda: _write_window_state(settings_path, window))
    return app, engine, window


def run(
    conn,
    data_root: str,
    runtime_config: dict[str, Any],
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> int:
    app, _engine, _window = build_application(
        conn,
        data_root,
        runtime_config,
        workspace_ref=workspace_ref,
        profile_ref=profile_ref,
    )
    return app.exec()


__all__ = ["build_application", "run"]
