"""Maintenance helpers for destructive local book reset flows."""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

from ..db import resolve_attachments_root
from ..errors import AppError
from .repo import current_context_snapshot


def _count_where(
    conn: sqlite3.Connection,
    table: str,
    where: str,
    params: tuple[Any, ...],
) -> int:
    row = conn.execute(
        f"SELECT COUNT(*) AS count FROM {table} WHERE {where}",
        params,
    ).fetchone()
    return int(row["count"] or 0)


def _count_profile_rows(conn: sqlite3.Connection, table: str, profile_id: str) -> int:
    return _count_where(conn, table, "profile_id = ?", (profile_id,))


def _count_sql(
    conn: sqlite3.Connection,
    query: str,
    params: tuple[Any, ...],
) -> int:
    row = conn.execute(query, params).fetchone()
    return int(row[0] or 0)


def _attachments_root_path(data_root: str) -> Path:
    return Path(resolve_attachments_root(data_root)).expanduser()


def _managed_attachment_paths_for_profile(
    conn: sqlite3.Connection,
    attachments_root: Path,
    profile_id: str,
) -> list[Path]:
    rows = conn.execute(
        """
        SELECT stored_relpath
        FROM attachments
        WHERE profile_id = ?
          AND stored_relpath IS NOT NULL
          AND stored_relpath != ''
        """,
        (profile_id,),
    ).fetchall()
    if not rows:
        return []
    root_resolved = attachments_root.resolve(strict=False)
    paths: list[Path] = []
    for row in rows:
        relpath = str(row["stored_relpath"] or "")
        candidate = (attachments_root / relpath).resolve(strict=False)
        try:
            candidate.relative_to(root_resolved)
        except ValueError:
            continue
        paths.append(candidate)
    return paths


def _prune_empty_dirs(root: Path, starting_path: Path):
    root_resolved = root.resolve(strict=False)
    current = starting_path.parent
    while current != root_resolved and current.is_dir():
        try:
            current.rmdir()
        except OSError:
            break
        current = current.parent


def _remove_reset_attachment_files(root: Path, paths: list[Path]) -> int:
    removed = 0
    for path in paths:
        try:
            if path.exists() and path.is_file():
                path.unlink()
                removed += 1
                _prune_empty_dirs(root, path)
        except OSError:
            continue
    return removed


def reset_current_profile_data(
    conn: sqlite3.Connection,
    data_root: str,
    *,
    clear_shared_rates: bool = False,
) -> dict[str, Any]:
    """Clear re-creatable data for the active book while preserving connections."""
    context = current_context_snapshot(conn)
    workspace_id = context.get("workspace_id")
    workspace_label = context.get("workspace_label")
    profile_id = context.get("profile_id")
    profile_label = context.get("profile_label")
    if not workspace_id or not profile_id:
        raise AppError(
            "No current book is selected.",
            code="state_not_ready",
            hint="Select a books set and book before resetting book data.",
        )

    attachments_root = _attachments_root_path(data_root)
    attachment_paths = _managed_attachment_paths_for_profile(
        conn,
        attachments_root,
        profile_id,
    )
    rates_cache_count = _count_where(conn, "rates_cache", "1 = 1", ())
    rates_checked_minutes_count = _count_where(
        conn,
        "rates_checked_minutes",
        "1 = 1",
        (),
    )
    removed = {
        "transactions": _count_profile_rows(conn, "transactions", profile_id),
        "transaction_tags": _count_sql(
            conn,
            """
            SELECT COUNT(*)
            FROM transaction_tags tt
            LEFT JOIN tags tag ON tag.id = tt.tag_id
            LEFT JOIN transactions tx ON tx.id = tt.transaction_id
            WHERE tag.profile_id = ? OR tx.profile_id = ?
            """,
            (profile_id, profile_id),
        ),
        "journal_entries": _count_profile_rows(conn, "journal_entries", profile_id),
        "journal_quarantines": _count_profile_rows(
            conn,
            "journal_quarantines",
            profile_id,
        ),
        "transaction_pairs": _count_profile_rows(
            conn,
            "transaction_pairs",
            profile_id,
        ),
        "transaction_pair_dismissals": _count_profile_rows(
            conn,
            "transaction_pair_dismissals",
            profile_id,
        ),
        "swap_matching_rules": _count_profile_rows(
            conn,
            "swap_matching_rules",
            profile_id,
        ),
        "saved_views": _count_profile_rows(conn, "saved_views", profile_id),
        "bip329_labels": _count_profile_rows(conn, "bip329_labels", profile_id),
        "attachments": _count_profile_rows(conn, "attachments", profile_id),
        "tags": _count_profile_rows(conn, "tags", profile_id),
        "source_funds_sources": _count_profile_rows(
            conn,
            "source_funds_sources",
            profile_id,
        ),
        "source_funds_links": _count_profile_rows(
            conn,
            "source_funds_links",
            profile_id,
        ),
        "source_funds_link_attachments": _count_sql(
            conn,
            """
            SELECT COUNT(*)
            FROM source_funds_link_attachments
            WHERE link_id IN (
                SELECT id FROM source_funds_links WHERE profile_id = ?
            )
            """,
            (profile_id,),
        ),
        "source_funds_source_attachments": _count_sql(
            conn,
            """
            SELECT COUNT(*)
            FROM source_funds_source_attachments
            WHERE source_id IN (
                SELECT id FROM source_funds_sources WHERE profile_id = ?
            )
            """,
            (profile_id,),
        ),
        "source_funds_cases": _count_profile_rows(
            conn,
            "source_funds_cases",
            profile_id,
        ),
        "source_funds_snapshots": _count_sql(
            conn,
            """
            SELECT COUNT(*)
            FROM source_funds_snapshots
            WHERE case_id IN (
                SELECT id FROM source_funds_cases WHERE profile_id = ?
            )
            """,
            (profile_id,),
        ),
        "source_funds_recipients": _count_profile_rows(
            conn,
            "source_funds_recipients",
            profile_id,
        ),
        "rates_cache": rates_cache_count if clear_shared_rates else 0,
        "rates_checked_minutes": (
            rates_checked_minutes_count if clear_shared_rates else 0
        ),
    }

    with conn:
        # These child-row deletes are explicit so the reset response can report
        # per-table counts; profile deletion would otherwise cascade them.
        conn.execute(
            """
            DELETE FROM source_funds_snapshots
            WHERE case_id IN (
                SELECT id FROM source_funds_cases WHERE profile_id = ?
            )
            """,
            (profile_id,),
        )
        conn.execute(
            """
            DELETE FROM source_funds_link_attachments
            WHERE link_id IN (
                SELECT id FROM source_funds_links WHERE profile_id = ?
            )
            """,
            (profile_id,),
        )
        conn.execute(
            """
            DELETE FROM source_funds_source_attachments
            WHERE source_id IN (
                SELECT id FROM source_funds_sources WHERE profile_id = ?
            )
            """,
            (profile_id,),
        )
        for table in (
            "source_funds_links",
            "source_funds_cases",
            "source_funds_sources",
            "source_funds_recipients",
            "saved_views",
            "swap_matching_rules",
            "transaction_pair_dismissals",
            "transaction_pairs",
            "bip329_labels",
            "journal_quarantines",
            "journal_entries",
            "attachments",
        ):
            conn.execute(f"DELETE FROM {table} WHERE profile_id = ?", (profile_id,))
        conn.execute(
            """
            DELETE FROM transaction_tags
            WHERE tag_id IN (SELECT id FROM tags WHERE profile_id = ?)
               OR transaction_id IN (
                   SELECT id FROM transactions WHERE profile_id = ?
               )
            """,
            (profile_id, profile_id),
        )
        conn.execute("DELETE FROM tags WHERE profile_id = ?", (profile_id,))
        conn.execute("DELETE FROM transactions WHERE profile_id = ?", (profile_id,))
        if clear_shared_rates:
            conn.execute("DELETE FROM rates_cache")
            conn.execute("DELETE FROM rates_checked_minutes")
        # Mark the profile as unprocessed so the next journal run fully
        # re-derives state from the preserved wallet/backend connections.
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = NULL,
                last_processed_tx_count = 0,
                last_processed_input_version = 0,
                journal_input_version = journal_input_version + 1
            WHERE id = ?
            """,
            (profile_id,),
        )

    removed["attachment_files"] = _remove_reset_attachment_files(
        attachments_root,
        attachment_paths,
    )
    preserved = {
        "workspaces": 1,
        "profiles": 1,
        "accounts": _count_profile_rows(conn, "accounts", profile_id),
        "wallets": _count_profile_rows(conn, "wallets", profile_id),
        "backends": _count_where(conn, "backends", "1 = 1", ()),
        "rates_cache": 0 if clear_shared_rates else rates_cache_count,
        "rates_checked_minutes": (
            0 if clear_shared_rates else rates_checked_minutes_count
        ),
    }
    return {
        "reset": True,
        "workspace": {"id": workspace_id, "label": workspace_label},
        "profile": {"id": profile_id, "label": profile_label},
        "preserved": preserved,
        "removed": removed,
        "rates_scope": "global" if clear_shared_rates else "preserved",
        "shared_rates_cleared": clear_shared_rates,
    }
