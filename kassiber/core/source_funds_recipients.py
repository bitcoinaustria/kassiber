"""Recipient profiles for source-of-funds disclosures.

A recipient represents the entity the user is preparing a disclosure for
(a tax authority, an exchange, a bank, a lawyer, ...). Recording it as
a first-class object enables three things the v1 workflow lacks:

1. **Sticky reveal-mode defaults.** Different recipients accept very
   different fields. A tax authority typically wants ``standard``;
   an exchange compliance form may insist on ``minimal`` to avoid
   leaking unrelated wallet history. Encoding the default once on the
   recipient prevents the user from picking the wrong reveal mode
   under stress.
2. **A audit trail of who got what.** When a saved case is exported,
   the case row records the recipient id. Future "did I already send
   this" lookups, or per-recipient channel logs, build on top of this.
3. **A starting point for recipient-shaped templates.** Field sets, PDF
   formats, and finding interpretations can later vary per recipient
   kind without changing the data model.

The module is deliberately minimal: CRUD plus a pair of helpers
(``resolve_recipient``, ``effective_reveal_mode``) that the export
path uses. UI surfaces and CLI subcommands wrap these primitives.
"""

from __future__ import annotations

import sqlite3
import uuid
from typing import Any, Mapping

from ..errors import AppError
from ..time_utils import now_iso
from .source_funds import REVEAL_MODES, _normalize_reveal_mode


RECIPIENT_KINDS = (
    "tax_authority",
    "exchange",
    "bank",
    "lawyer",
    "accountant",
    "other",
)


def _normalize_kind(value: str | None) -> str:
    kind = (value or "").strip().lower().replace("-", "_")
    if kind not in RECIPIENT_KINDS:
        raise AppError(
            f"Unsupported recipient kind '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(RECIPIENT_KINDS)}",
        )
    return kind


def _row_to_dict(row: Mapping[str, Any]) -> dict[str, Any]:
    keys = row.keys()
    active = bool(row["active"]) if "active" in keys else True
    return {
        "id": row["id"],
        "label": row["label"],
        "kind": row["kind"],
        "default_reveal_mode": row["default_reveal_mode"],
        "notes": row["notes"] or "",
        "active": active,
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def create_recipient(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    *,
    label: str,
    kind: str,
    default_reveal_mode: str = "standard",
    notes: str | None = None,
) -> dict[str, Any]:
    """Create a recipient row.

    The (profile_id, label) UNIQUE constraint is the primary integrity
    check: each profile can only have one recipient with a given label.
    """
    label = (label or "").strip()
    if not label:
        raise AppError("Recipient label is required", code="validation")
    normalized_kind = _normalize_kind(kind)
    normalized_reveal = _normalize_reveal_mode(default_reveal_mode)
    notes_value = (notes or "").strip() or None
    recipient_id = str(uuid.uuid4())
    timestamp = now_iso()
    try:
        conn.execute(
            """
            INSERT INTO source_funds_recipients(id, workspace_id, profile_id, label, kind,
                default_reveal_mode, notes, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                recipient_id,
                workspace_id,
                profile_id,
                label,
                normalized_kind,
                normalized_reveal,
                notes_value,
                timestamp,
                timestamp,
            ),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"A recipient labelled '{label}' already exists in this profile.",
            code="validation",
        ) from exc
    conn.commit()
    return get_recipient(conn, profile_id, recipient_id)


def list_recipients(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    include_inactive: bool = False,
) -> list[dict[str, Any]]:
    if include_inactive:
        rows = conn.execute(
            """
            SELECT *
            FROM source_funds_recipients
            WHERE profile_id = ?
            ORDER BY active DESC, label COLLATE NOCASE ASC, id ASC
            """,
            (profile_id,),
        ).fetchall()
    else:
        rows = conn.execute(
            """
            SELECT *
            FROM source_funds_recipients
            WHERE profile_id = ? AND active = 1
            ORDER BY label COLLATE NOCASE ASC, id ASC
            """,
            (profile_id,),
        ).fetchall()
    return [_row_to_dict(row) for row in rows]


def get_recipient(
    conn: sqlite3.Connection,
    profile_id: str,
    recipient_id: str,
) -> dict[str, Any]:
    row = conn.execute(
        "SELECT * FROM source_funds_recipients WHERE profile_id = ? AND id = ?",
        (profile_id, recipient_id),
    ).fetchone()
    if row is None:
        raise AppError(
            f"Recipient '{recipient_id}' not found in this profile.",
            code="not_found",
        )
    return _row_to_dict(row)


def resolve_recipient(
    conn: sqlite3.Connection,
    profile_id: str,
    ref: str,
) -> dict[str, Any]:
    """Resolve a recipient by id or label, like other ``resolve_*`` helpers.

    Looking up by label is the path the desktop UI takes when the user
    picks from a dropdown. Looking up by id is the path the export gate
    takes after a previous step persisted it.
    """
    ref_value = (ref or "").strip()
    if not ref_value:
        raise AppError("Recipient ref is required", code="validation")
    by_id = conn.execute(
        "SELECT * FROM source_funds_recipients WHERE profile_id = ? AND id = ?",
        (profile_id, ref_value),
    ).fetchone()
    if by_id is not None:
        return _row_to_dict(by_id)
    by_label = conn.execute(
        "SELECT * FROM source_funds_recipients WHERE profile_id = ? AND label = ? AND active = 1",
        (profile_id, ref_value),
    ).fetchone()
    if by_label is not None:
        return _row_to_dict(by_label)
    raise AppError(
        f"Recipient '{ref_value}' not found in this profile.",
        code="not_found",
    )


def update_recipient(
    conn: sqlite3.Connection,
    profile_id: str,
    recipient_id: str,
    *,
    label: str | None = None,
    kind: str | None = None,
    default_reveal_mode: str | None = None,
    notes: str | None = None,
) -> dict[str, Any]:
    existing = get_recipient(conn, profile_id, recipient_id)
    new_label = label.strip() if isinstance(label, str) else existing["label"]
    if not new_label:
        raise AppError("Recipient label cannot be empty", code="validation")
    new_kind = _normalize_kind(kind) if kind is not None else existing["kind"]
    new_reveal = (
        _normalize_reveal_mode(default_reveal_mode)
        if default_reveal_mode is not None
        else existing["default_reveal_mode"]
    )
    if notes is None:
        new_notes: str | None = existing["notes"] or None
    else:
        new_notes = notes.strip() or None
    timestamp = now_iso()
    try:
        conn.execute(
            """
            UPDATE source_funds_recipients
            SET label = ?, kind = ?, default_reveal_mode = ?, notes = ?, updated_at = ?
            WHERE profile_id = ? AND id = ?
            """,
            (new_label, new_kind, new_reveal, new_notes, timestamp, profile_id, recipient_id),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"A recipient labelled '{new_label}' already exists in this profile.",
            code="validation",
        ) from exc
    conn.commit()
    return get_recipient(conn, profile_id, recipient_id)


def delete_recipient(
    conn: sqlite3.Connection,
    profile_id: str,
    recipient_id: str,
) -> dict[str, Any]:
    """Soft-delete a recipient.

    Hard delete would erase the stable ``recipient_id`` foreign key on
    saved disclosure cases, which means a future audit cannot answer
    "was this case sent to recipient id X?" - even if the snapshot
    label/kind survive, the unique identifier is gone, and a new
    recipient created later with the same label is indistinguishable
    from the deleted one.

    Soft delete preserves the row (and the FK target) but marks it
    ``active = 0`` so it disappears from default lists/UI pickers.

    Saved cases reference the recipient by id but already snapshot the
    label/kind/default_reveal_mode at save time, so renaming or
    soft-deleting a recipient does not retroactively rewrite history -
    the snapshot is the contract.
    """
    get_recipient(conn, profile_id, recipient_id)  # raise not_found early
    timestamp = now_iso()
    conn.execute(
        "UPDATE source_funds_recipients SET active = 0, updated_at = ? WHERE profile_id = ? AND id = ?",
        (timestamp, profile_id, recipient_id),
    )
    conn.commit()
    refreshed = get_recipient(conn, profile_id, recipient_id)
    return refreshed


def restore_recipient(
    conn: sqlite3.Connection,
    profile_id: str,
    recipient_id: str,
) -> dict[str, Any]:
    """Re-activate a soft-deleted recipient.

    Pairs with :func:`delete_recipient`. Fails the partial unique index
    if the user already created a new recipient with the same label
    after the soft delete.
    """
    existing = get_recipient(conn, profile_id, recipient_id)
    if existing["active"]:
        return existing
    timestamp = now_iso()
    try:
        conn.execute(
            "UPDATE source_funds_recipients SET active = 1, updated_at = ? WHERE profile_id = ? AND id = ?",
            (timestamp, profile_id, recipient_id),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"A recipient labelled '{existing['label']}' already exists in this profile.",
            code="validation",
            hint="Rename or remove the existing recipient before restoring this one.",
        ) from exc
    conn.commit()
    return get_recipient(conn, profile_id, recipient_id)


def effective_reveal_mode(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    explicit_reveal_mode: str | None,
    recipient_ref: str | None,
) -> tuple[str, dict[str, Any] | None]:
    """Resolve the reveal mode to use, applying recipient sticky defaults.

    Precedence:
        1. ``explicit_reveal_mode`` if the caller passed one (other than
           the legacy default ``"standard"``-as-fallback - callers
           that want sticky defaults pass ``None``).
        2. The recipient's ``default_reveal_mode`` if a recipient was
           given.
        3. ``"standard"``.

    Returns ``(reveal_mode, recipient_or_none)`` so the caller can both
    use the resolved mode and persist the resolved recipient row.
    """
    recipient: dict[str, Any] | None = None
    if recipient_ref:
        recipient = resolve_recipient(conn, profile_id, recipient_ref)
    if explicit_reveal_mode:
        return _normalize_reveal_mode(explicit_reveal_mode), recipient
    if recipient is not None:
        return _normalize_reveal_mode(recipient["default_reveal_mode"]), recipient
    return "standard", recipient


def supported_kinds() -> tuple[str, ...]:
    return RECIPIENT_KINDS


def supported_reveal_modes() -> tuple[str, ...]:
    return REVEAL_MODES
