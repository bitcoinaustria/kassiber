from __future__ import annotations

from ...errors import AppError


def resolve_wallet(conn, profile_id, ref):
    normalized_ref = str(ref).strip()
    row = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND w.id = ?
        LIMIT 1
        """,
        (profile_id, normalized_ref),
    ).fetchone()
    if row:
        return row
    rows = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND lower(w.label) = lower(?)
        ORDER BY w.label ASC, w.id ASC
        """,
        (profile_id, normalized_ref),
    ).fetchall()
    if len(rows) == 1:
        return rows[0]
    if len(rows) > 1:
        raise AppError(
            f"Wallet label '{ref}' is ambiguous",
            code="validation",
            hint="Use the wallet id instead of the non-unique label.",
            details={
                "matches": [
                    {
                        "id": row["id"],
                        "label": row["label"],
                        "account_code": row["account_code"],
                    }
                    for row in rows
                ]
            },
        )
    raise AppError(f"Wallet '{ref}' not found", code="not_found")


def fetch_wallet_with_account(conn, wallet_id):
    return conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.id = ?
        """,
        (wallet_id,),
    ).fetchone()


def wallet_transaction_count(conn, wallet_id):
    row = conn.execute(
        "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = ?",
        (wallet_id,),
    ).fetchone()
    return int(row["n"])
