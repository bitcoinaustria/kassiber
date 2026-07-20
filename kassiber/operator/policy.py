"""Authenticated binding for path-adjacent operator policy and credentials."""

from __future__ import annotations

from dataclasses import dataclass

from ..db import (
    load_managed_settings,
    mutate_managed_settings,
    resolve_canonical_project_data_root,
)
from ..errors import AppError
from .project import canonical_project


OPERATOR_POLICY_PROJECT_IDENTITY_SETTING = "operator_policy_project_identity"
OPERATOR_POLICY_DATABASE_IDENTITY_SETTING = "operator_policy_database_identity"
_PERMISSIVE_POLICY_KEYS = frozenset(
    {
        "operator_unlock_mode",
        "cli_remembered_unlock",
        "cli_remembered_unlock_enrollment",
    }
)


@dataclass(frozen=True)
class ProjectPolicyBinding:
    data_root: str
    project_identity: str
    database_identity: str


@dataclass(frozen=True)
class ProjectPolicyState:
    data_root: str
    project_identity: str
    binding_state: str
    stored_database_identity: str | None


def require_project_policy_binding(data_root) -> ProjectPolicyBinding:
    """Return a binding only when it still names the current database file."""

    canonical_root = str(resolve_canonical_project_data_root(data_root))
    project = canonical_project(canonical_root)
    settings = load_managed_settings(canonical_root)
    stored_project = settings.get(OPERATOR_POLICY_PROJECT_IDENTITY_SETTING)
    stored_database = settings.get(OPERATOR_POLICY_DATABASE_IDENTITY_SETTING)
    if not _valid_project_identity(stored_project) or not _valid_database_identity(
        stored_database
    ):
        raise AppError(
            "the project unlock policy is not bound to an authenticated database",
            code="operator_policy_binding_required",
            hint=(
                "Authenticate with `kassiber operator mode` or re-enroll remembered "
                "unlock before using a reusable unlock policy."
            ),
            retryable=False,
        )
    if stored_project != project.identity:
        raise AppError(
            "the project database no longer matches its unlock policy",
            code="operator_policy_binding_mismatch",
            hint=(
                "Use manual password authorization, then select the intended "
                "operator mode again for this database."
            ),
            retryable=False,
        )
    return ProjectPolicyBinding(
        data_root=canonical_root,
        project_identity=stored_project,
        database_identity=stored_database,
    )


def project_policy_state(data_root) -> ProjectPolicyState:
    """Return categorical binding state without exposing either raw identity."""

    canonical_root = str(resolve_canonical_project_data_root(data_root))
    project = canonical_project(canonical_root)
    settings = load_managed_settings(canonical_root)
    stored_project = settings.get(OPERATOR_POLICY_PROJECT_IDENTITY_SETTING)
    stored_database = settings.get(OPERATOR_POLICY_DATABASE_IDENTITY_SETTING)
    if not _valid_project_identity(stored_project) or not _valid_database_identity(
        stored_database
    ):
        binding_state = "missing"
        stored_database = None
    elif stored_project != project.identity:
        binding_state = "mismatch"
    else:
        binding_state = "valid"
    return ProjectPolicyState(
        data_root=canonical_root,
        project_identity=project.identity,
        binding_state=binding_state,
        stored_database_identity=stored_database,
    )


def bind_project_policy(
    data_root,
    database_identity: str,
    *,
    expected_project_identity: str | None = None,
    updates: dict[str, object] | None = None,
    remove: tuple[str, ...] = (),
) -> ProjectPolicyBinding:
    """Bind policy state after an authenticated database open."""

    if not _valid_database_identity(database_identity):
        raise AppError(
            "the authenticated database identity is invalid",
            code="invalid_project_database",
            retryable=False,
        )
    canonical_root = str(resolve_canonical_project_data_root(data_root))
    project = canonical_project(canonical_root)
    if (
        expected_project_identity is not None
        and project.identity != expected_project_identity
    ):
        raise AppError(
            "the project changed before its unlock policy could be bound",
            code="operator_project_replaced",
            retryable=False,
        )

    def mutate(payload: dict[str, object]) -> dict[str, object]:
        rebinding = (
            payload.get(OPERATOR_POLICY_PROJECT_IDENTITY_SETTING)
            != project.identity
            or payload.get(OPERATOR_POLICY_DATABASE_IDENTITY_SETTING)
            != database_identity
        )
        if rebinding:
            for key in _PERMISSIVE_POLICY_KEYS:
                payload.pop(key, None)
        payload[OPERATOR_POLICY_PROJECT_IDENTITY_SETTING] = project.identity
        payload[OPERATOR_POLICY_DATABASE_IDENTITY_SETTING] = database_identity
        for key, value in (updates or {}).items():
            payload[str(key)] = value
        for key in remove:
            payload.pop(str(key), None)
        return payload

    mutate_managed_settings(
        canonical_root,
        mutate,
    )
    if canonical_project(canonical_root).identity != project.identity:
        raise AppError(
            "the project changed while its unlock policy was being bound",
            code="operator_project_replaced",
            retryable=False,
        )
    return ProjectPolicyBinding(
        data_root=canonical_root,
        project_identity=project.identity,
        database_identity=database_identity,
    )


def stored_project_policy_database_identity(data_root) -> str | None:
    """Return a syntactically valid stored identity for cleanup only."""

    canonical_root = str(resolve_canonical_project_data_root(data_root))
    value = load_managed_settings(canonical_root).get(
        OPERATOR_POLICY_DATABASE_IDENTITY_SETTING
    )
    return value if _valid_database_identity(value) else None


def _valid_project_identity(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def _valid_database_identity(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdef" for character in value)
    )
