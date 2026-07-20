"""Explicit project unlock modes and the one-time legacy migration rule."""

from __future__ import annotations

from typing import Literal, cast

from ..db import (
    load_managed_settings,
    resolve_canonical_project_data_root,
    update_managed_settings,
)
from ..errors import AppError
from ..secrets.unlock_store import cli_remembered_unlock_enabled


UnlockMode = Literal["manual", "brokered", "unattended"]
UNLOCK_MODES = frozenset({"manual", "brokered", "unattended"})
OPERATOR_UNLOCK_MODE_SETTING = "operator_unlock_mode"


def configured_unlock_mode(data_root) -> UnlockMode | None:
    """Return the explicit mode, or None for installations not yet migrated."""

    data_root = resolve_canonical_project_data_root(data_root)
    value = load_managed_settings(data_root).get(OPERATOR_UNLOCK_MODE_SETTING)
    if value is None:
        return None
    if not isinstance(value, str) or value not in UNLOCK_MODES:
        raise AppError(
            "the configured operator unlock mode is invalid",
            code="invalid_operator_unlock_mode",
            hint="Choose manual, brokered, or unattended with `kassiber operator mode`.",
            details={"configured": str(value)},
            retryable=False,
        )
    return cast(UnlockMode, value)


def effective_unlock_mode(data_root) -> UnlockMode:
    """Resolve the fail-closed default plus the legacy remembered-store bridge."""

    data_root = resolve_canonical_project_data_root(data_root)
    configured = configured_unlock_mode(data_root)
    if configured is not None:
        return configured
    if cli_remembered_unlock_enabled(data_root):
        return "unattended"
    return "manual"


def unlock_mode_status(data_root) -> dict[str, object]:
    data_root = resolve_canonical_project_data_root(data_root)
    configured = configured_unlock_mode(data_root)
    effective = effective_unlock_mode(data_root)
    return {
        "configured": configured,
        "effective": effective,
        "legacy_inferred": configured is None and effective == "unattended",
    }


def set_unlock_mode(data_root, mode: str) -> UnlockMode:
    if mode not in UNLOCK_MODES:
        raise AppError(
            f"unsupported operator unlock mode: {mode}",
            code="invalid_operator_unlock_mode",
            details={"allowed": sorted(UNLOCK_MODES)},
            retryable=False,
        )
    update_managed_settings(
        resolve_canonical_project_data_root(data_root),
        updates={OPERATOR_UNLOCK_MODE_SETTING: mode},
    )
    return cast(UnlockMode, mode)


def remembered_unlock_allowed(data_root) -> bool:
    """Remembered credentials are consulted only in unattended mode."""

    return (
        effective_unlock_mode(resolve_canonical_project_data_root(data_root))
        == "unattended"
    )
