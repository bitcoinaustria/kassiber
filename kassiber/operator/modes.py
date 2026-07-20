"""Explicit project unlock modes and the one-time legacy migration rule."""

from __future__ import annotations

from typing import Literal, cast

from ..db import (
    load_managed_settings,
    resolve_canonical_project_data_root,
    update_managed_settings,
)
from ..errors import AppError
from .policy import (
    bind_project_policy,
    project_policy_state,
    require_project_policy_binding,
)


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
    """Resolve the configured mode with a fail-closed manual default."""

    data_root = resolve_canonical_project_data_root(data_root)
    configured = configured_unlock_mode(data_root)
    if configured is not None:
        if configured != "manual":
            require_project_policy_binding(data_root)
        return configured
    return "manual"


def unlock_mode_status(data_root) -> dict[str, object]:
    data_root = resolve_canonical_project_data_root(data_root)
    configured = configured_unlock_mode(data_root)
    binding_state = project_policy_state(data_root).binding_state
    try:
        effective = effective_unlock_mode(data_root)
    except AppError as exc:
        if exc.code not in {
            "operator_policy_binding_required",
            "operator_policy_binding_mismatch",
        }:
            raise
        effective = "manual"
    return {
        "configured": configured,
        "effective": effective,
        "legacy_inferred": False,
        "binding_state": binding_state,
    }


def set_unlock_mode(
    data_root,
    mode: str,
    *,
    database_identity: str | None = None,
    expected_project_identity: str | None = None,
) -> UnlockMode:
    if mode not in UNLOCK_MODES:
        raise AppError(
            f"unsupported operator unlock mode: {mode}",
            code="invalid_operator_unlock_mode",
            details={"allowed": sorted(UNLOCK_MODES)},
            retryable=False,
        )
    canonical_root = str(resolve_canonical_project_data_root(data_root))
    if database_identity is not None:
        bind_project_policy(
            canonical_root,
            database_identity,
            expected_project_identity=expected_project_identity,
            updates={OPERATOR_UNLOCK_MODE_SETTING: mode},
        )
        return cast(UnlockMode, mode)
    if mode != "manual":
        raise AppError(
            "a reusable unlock mode requires an authenticated database binding",
            code="operator_policy_binding_required",
            retryable=False,
        )
    # A restrictive manual policy remains a safe recovery state when old
    # path-adjacent settings cannot yet be authenticated and rebound.
    update_managed_settings(canonical_root, updates={OPERATOR_UNLOCK_MODE_SETTING: mode})
    return cast(UnlockMode, mode)


def remembered_unlock_allowed(data_root) -> bool:
    """Remembered credentials are consulted only in unattended mode."""

    return (
        effective_unlock_mode(resolve_canonical_project_data_root(data_root))
        == "unattended"
    )
