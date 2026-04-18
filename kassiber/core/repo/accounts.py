from __future__ import annotations

from ...errors import AppError


def resolve_account(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Account '{ref}' not found")
    return row
