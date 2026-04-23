from __future__ import annotations

from ...errors import AppError


def resolve_account(conn, profile_id, ref):
    normalized_ref = str(ref).strip()
    row = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND id = ?
        LIMIT 1
        """,
        (profile_id, normalized_ref),
    ).fetchone()
    if row:
        return row

    row = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND lower(code) = lower(?)
        LIMIT 1
        """,
        (profile_id, normalized_ref),
    ).fetchone()
    if row:
        return row

    rows = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND lower(label) = lower(?)
        ORDER BY code ASC
        """,
        (profile_id, normalized_ref),
    ).fetchall()
    if len(rows) == 1:
        return rows[0]
    if len(rows) > 1:
        raise AppError(
            f"Account label '{ref}' is ambiguous",
            code="validation",
            hint="Use the account code or id instead of the non-unique label.",
            details={
                "matches": [
                    {
                        "id": row["id"],
                        "code": row["code"],
                        "label": row["label"],
                    }
                    for row in rows
                ]
            },
        )
    raise AppError(f"Account '{ref}' not found", code="not_found")
