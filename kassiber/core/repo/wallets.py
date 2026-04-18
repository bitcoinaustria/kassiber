from __future__ import annotations

from ...errors import AppError


def resolve_wallet(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND (w.id = ? OR lower(w.label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Wallet '{ref}' not found")
    return row


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
