"""Mandatory freshness and custody gate for journal-derived reports."""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from typing import Any, Callable, Mapping

from ..errors import AppError
from ..tax_policy import require_tax_processing_supported
from . import custody_journal
from . import custody_quantity_store


ScopeResolver = Callable[
    [sqlite3.Connection, Any, Any],
    tuple[Mapping[str, Any], Mapping[str, Any]],
]
ProfileValidator = Callable[[Mapping[str, Any]], None]


def _row_int(row: Mapping[str, Any], key: str) -> int:
    try:
        value = row[key]
    except (IndexError, KeyError):
        value = None
    return int(value or 0)


@dataclass(frozen=True)
class ReportContext:
    """Proof that one profile's stored journal projection passed its gate."""

    workspace: Mapping[str, Any]
    profile: Mapping[str, Any]
    active_transaction_count: int
    journal_input_version: int
    last_processed_input_version: int
    last_processed_at: str

    @property
    def workspace_id(self) -> str:
        return str(self.workspace["id"])

    @property
    def profile_id(self) -> str:
        return str(self.profile["id"])


def require_report_context(
    conn: sqlite3.Connection,
    workspace_ref: Any,
    profile_ref: Any,
    resolve_scope: ScopeResolver,
    *,
    validate_profile: ProfileValidator | None = None,
) -> ReportContext:
    """Resolve scope once and fail closed unless its projection is reportable."""

    workspace, resolved_profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(resolved_profile)
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (resolved_profile["id"],),
    ).fetchone()
    if profile is None:
        raise AppError("Report profile was not found.", code="not_found")
    if validate_profile is not None:
        validate_profile(profile)

    component_blockers = custody_journal.component_integrity_blockers(
        conn, profile["id"]
    )
    if component_blockers:
        raise AppError(
            "Reports are blocked by an incomplete or conflicting custody component.",
            code="custody_component_incomplete",
            hint=(
                "Repair or supersede every authored active component in "
                "`kassiber transfers components list` before relying on reports."
            ),
            details={"components": component_blockers},
            retryable=False,
        )

    quantity_issues = custody_quantity_store.blocking_quantity_issues(
        conn, profile["id"]
    )
    if quantity_issues:
        blocked_from = next(
            (
                str(item["blocks_from"])
                for item in quantity_issues
                if item.get("blocks_from")
            ),
            None,
        )
        raise AppError(
            "Reports are blocked by unresolved custody quantity.",
            code="custody_quantity_unresolved",
            hint=(
                "Review the custody quantity issues before relying on tax or "
                "portfolio reports. Later basis is not final."
            ),
            details={"blocked_from": blocked_from, "issues": quantity_issues[:20]},
            retryable=False,
        )

    active_count = int(
        conn.execute(
            "SELECT COUNT(*) FROM transactions "
            "WHERE profile_id = ? AND excluded = 0",
            (profile["id"],),
        ).fetchone()[0]
        or 0
    )
    input_version = _row_int(profile, "journal_input_version")
    processed_version = _row_int(profile, "last_processed_input_version")
    processed_at = str(profile["last_processed_at"] or "")
    if not (
        processed_at
        and active_count == _row_int(profile, "last_processed_tx_count")
        and input_version == processed_version
    ):
        raise AppError(
            "Reports require fresh journals. Run `kassiber journals process` first."
        )

    return ReportContext(
        workspace=workspace,
        profile=profile,
        active_transaction_count=active_count,
        journal_input_version=input_version,
        last_processed_input_version=processed_version,
        last_processed_at=processed_at,
    )
