"""Loan marks: tag a transaction as Bitcoin-backed-loan collateral or principal.

A *collateral lock* (an outbound posting BTC as loan collateral) is not a
disposal — the borrower still owns the coins, just encumbered. The matching
*collateral release* (the inbound when the collateral returns on repayment) is
not an acquisition: the coins re-enter the pool they never really left, so a
lock/release round-trip nets to zero and preserves the original basis and
acquisition date (Alt/Neu by date, not a hold period).

A *principal received* leg (inbound borrowed BTC) is not income or an
acquisition of owned coins. A *principal repaid* leg (outbound returned BTC) is
not a disposal of owned coins. These principal roles model the loan liability
principal only; interest, liquidation, and platform-specific accounting remain
outside this minimal per-transaction mark.

Deliberately minimal. A mark is one row in ``loan_legs`` linking a journal
transaction to a role. Related marks can share a lightweight ``loan_id`` for UI
and audit readability, but there is no facility record, and no custody,
rehypothecation, interest, or liquidation modelling. If collateral is
liquidated and never returns, the user removes the lock mark and the outbound
reverts to the normal disposal it always was (surfaced by ``open_collateral_locks``
as a reconcile hint). Watching for the liquidation itself is the loan platform's
job; booking its tax consequence is the user's, via that un-mark. The tax engine
consumes ``load_collateral_role_map`` to classify the matching transactions — the
role, never an address shape, decides taxability.
"""

from __future__ import annotations

import uuid
from typing import Any, Optional

from ..errors import AppError
from ..time_utils import now_iso

# Loan leg roles. Outbound roles suppress disposal booking; inbound roles
# suppress acquisition/income booking. The strings are also the membership values
# the tax engine checks, so keep them stable.
COLLATERAL_LOCK = "collateral_lock"
COLLATERAL_RELEASE = "collateral_release"
PRINCIPAL_RECEIVED = "loan_principal_received"
PRINCIPAL_REPAID = "loan_principal_repaid"
COLLATERAL_ROLES = (
    COLLATERAL_LOCK,
    COLLATERAL_RELEASE,
    PRINCIPAL_RECEIVED,
    PRINCIPAL_REPAID,
)

# Lightning channel-lifecycle roles. Opening a channel moves the operator's own
# BTC from their on-chain wallet into a 2-of-2 they co-control (still owned —
# not a disposal); closing returns it (not an acquisition — basis carries). Same
# suppress semantics as loan collateral lock/release, but these are DERIVED from
# channel funding/closing txids, never user-marked, so they stay out of
# ``COLLATERAL_ROLES`` (the loan-mark validator's allow-list).
CHANNEL_OPEN = "channel_open"
CHANNEL_CLOSE = "channel_close"
CHANNEL_ROLES = (CHANNEL_OPEN, CHANNEL_CLOSE)
# A funding tx whose recorded outflow clearly exceeds the channel's funded
# amount also paid an external recipient. Suppressing the whole row would
# silently untax that payment, so the row is quarantined for explicit review
# instead (NOT in the suppress sets below).
CHANNEL_OPEN_MISMATCH = "channel_open_mismatch"

ROLE_DIRECTIONS = {
    COLLATERAL_LOCK: "outbound",
    COLLATERAL_RELEASE: "inbound",
    PRINCIPAL_RECEIVED: "inbound",
    PRINCIPAL_REPAID: "outbound",
    CHANNEL_OPEN: "outbound",
    CHANNEL_CLOSE: "inbound",
}

# Outbound legs that are non-events: suppress disposal booking.
LOCK_SUPPRESS_ROLES = frozenset({COLLATERAL_LOCK, PRINCIPAL_REPAID, CHANNEL_OPEN})
# Inbound legs that are non-events: suppress acquisition/income booking.
RELEASE_SUPPRESS_ROLES = frozenset(
    {COLLATERAL_RELEASE, PRINCIPAL_RECEIVED, CHANNEL_CLOSE}
)

# Human labels for the CLI / GUI / reconcile hints.
ROLE_LABELS = {
    COLLATERAL_LOCK: "BTC collateral posted for fiat loan (out)",
    COLLATERAL_RELEASE: "BTC collateral returned (in)",
    PRINCIPAL_RECEIVED: "BTC loan principal received (in)",
    PRINCIPAL_REPAID: "BTC loan principal repaid (out)",
    CHANNEL_OPEN: "BTC funded into a Lightning channel (out)",
    CHANNEL_CLOSE: "BTC returned from a Lightning channel close (in)",
}


def _require_role(role: str) -> None:
    if role not in COLLATERAL_ROLES:
        raise AppError(
            f"Invalid loan role '{role}'. Allowed: {', '.join(COLLATERAL_ROLES)}",
            code="validation",
            details={"field": "role", "value": role},
        )


def _require_role_direction(role: str, direction: str) -> None:
    expected = ROLE_DIRECTIONS[role]
    if direction != expected:
        raise AppError(
            f"Loan role '{role}' requires an {expected} transaction",
            code="validation",
            details={
                "field": "role",
                "role": role,
                "expected_direction": expected,
                "actual_direction": direction,
            },
        )


def mark_collateral(
    conn,
    workspace_id: str,
    profile_id: str,
    transaction_id: str,
    *,
    role: str,
    note: Optional[str] = None,
    loan_id: Optional[str] = None,
    commit: bool = True,
) -> dict[str, Any]:
    """Mark a transaction as a loan role. One active mark per transaction —
    re-marking replaces the existing role."""
    _require_role(role)
    tx = conn.execute(
        "SELECT id, direction FROM transactions WHERE id = ? AND profile_id = ?",
        (transaction_id, profile_id),
    ).fetchone()
    if tx is None:
        raise AppError(
            f"Transaction '{transaction_id}' not found in this profile",
            code="not_found",
            details={"transaction_id": transaction_id},
        )
    _require_role_direction(role, str(tx["direction"]))
    existing = conn.execute(
        "SELECT loan_id FROM loan_legs WHERE profile_id = ? AND transaction_id = ? AND deleted_at IS NULL",
        (profile_id, transaction_id),
    ).fetchone()
    effective_loan_id = (loan_id or "").strip() or (
        str(existing["loan_id"]) if existing is not None and existing["loan_id"] else None
    )
    # Re-mark: retire any existing active mark for this transaction first so the
    # one-active-mark-per-transaction unique index is never violated.
    conn.execute(
        "UPDATE loan_legs SET deleted_at = ? WHERE profile_id = ? AND transaction_id = ? AND deleted_at IS NULL",
        (now_iso(), profile_id, transaction_id),
    )
    leg_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO loan_legs(id, workspace_id, profile_id, transaction_id, loan_id, role, note, deleted_at, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (leg_id, workspace_id, profile_id, transaction_id, effective_loan_id, role, note, now_iso()),
    )
    if commit:
        conn.commit()
    row = conn.execute("SELECT * FROM loan_legs WHERE id = ?", (leg_id,)).fetchone()
    return {key: row[key] for key in row.keys()}


def unmark_collateral(
    conn, profile_id: str, transaction_id: str, *, commit: bool = True
) -> dict[str, Any]:
    """Remove the loan mark from a transaction. The transaction reverts to its
    normal tax classification."""
    row = conn.execute(
        "SELECT id FROM loan_legs WHERE profile_id = ? AND transaction_id = ? AND deleted_at IS NULL",
        (profile_id, transaction_id),
    ).fetchone()
    if row is None:
        raise AppError(
            f"No loan mark on transaction '{transaction_id}'",
            code="not_found",
            details={"transaction_id": transaction_id},
        )
    conn.execute(
        "UPDATE loan_legs SET deleted_at = ? WHERE id = ?",
        (now_iso(), row["id"]),
    )
    if commit:
        conn.commit()
    return {"unmarked": transaction_id}


def link_loan_marks(
    conn,
    profile_id: str,
    transaction_ids: list[str],
    *,
    loan_id: Optional[str] = None,
    commit: bool = True,
) -> dict[str, Any]:
    """Assign the same lightweight loan id to two or more active loan marks."""
    unique_ids = list(dict.fromkeys(transaction_ids))
    if len(unique_ids) < 2:
        raise AppError(
            "At least two marked loan transactions are required to link a loan",
            code="validation",
            details={"transaction_ids": unique_ids},
        )
    placeholders = ", ".join("?" for _ in unique_ids)
    rows = conn.execute(
        f"""
        SELECT transaction_id, loan_id
        FROM loan_legs
        WHERE profile_id = ?
          AND deleted_at IS NULL
          AND transaction_id IN ({placeholders})
        """,
        (profile_id, *unique_ids),
    ).fetchall()
    rows_by_transaction_id = {str(row["transaction_id"]): row for row in rows}
    found = set(rows_by_transaction_id)
    missing = [transaction_id for transaction_id in unique_ids if transaction_id not in found]
    if missing:
        raise AppError(
            "All linked loan transactions must already have a loan mark",
            code="not_found",
            details={"transaction_ids": unique_ids, "missing": missing},
        )
    chosen_loan_id = (loan_id or "").strip() or next(
        (
            str(rows_by_transaction_id[transaction_id]["loan_id"])
            for transaction_id in unique_ids
            if rows_by_transaction_id[transaction_id]["loan_id"]
        ),
        str(uuid.uuid4()),
    )
    conn.execute(
        f"""
        UPDATE loan_legs
        SET loan_id = ?
        WHERE profile_id = ?
          AND deleted_at IS NULL
          AND transaction_id IN ({placeholders})
        """,
        (chosen_loan_id, profile_id, *unique_ids),
    )
    if commit:
        conn.commit()
    return {"loan_id": chosen_loan_id, "transaction_ids": unique_ids}


def load_collateral_role_map(conn, profile_id: str) -> dict[str, str]:
    """``{transaction_id: role}`` for active loan marks — consumed by the tax
    pipeline to classify the matching journal transaction by its role."""
    rows = conn.execute(
        "SELECT transaction_id, role FROM loan_legs WHERE profile_id = ? AND deleted_at IS NULL",
        (profile_id,),
    ).fetchall()
    return {str(row["transaction_id"]): str(row["role"]) for row in rows}


def list_collateral_marks(conn, profile_id: str) -> list[dict[str, Any]]:
    """All active loan marks joined to their transaction, newest first."""
    rows = conn.execute(
        """
        SELECT ll.transaction_id, ll.loan_id, ll.role, ll.note, ll.created_at,
               t.direction, t.asset, t.amount, t.occurred_at, t.description
        FROM loan_legs ll
        JOIN transactions t ON t.id = ll.transaction_id
        WHERE ll.profile_id = ? AND ll.deleted_at IS NULL
        ORDER BY t.occurred_at DESC, ll.created_at DESC
        """,
        (profile_id,),
    ).fetchall()
    return [{key: row[key] for key in row.keys()} for row in rows]


def open_collateral_locks(conn, profile_id: str) -> list[dict[str, Any]]:
    """Locks with no offsetting release — collateral that left and hasn't come
    back. Drives the reconcile hint: confirm repaid (add a release) or liquidated
    (remove the lock so it books as a disposal). Heuristic, per-asset: a lock is
    'open' until matched one-for-one by a release of the same asset. Signal only,
    never a tax decision."""
    marks = list_collateral_marks(conn, profile_id)
    releases_by_asset: dict[str, int] = {}
    for mark in marks:
        if mark["role"] == COLLATERAL_RELEASE:
            releases_by_asset[mark["asset"]] = releases_by_asset.get(mark["asset"], 0) + 1
    open_locks: list[dict[str, Any]] = []
    # Oldest first so the longest-outstanding lock is matched (and surfaced) first.
    for mark in sorted(marks, key=lambda m: (m["occurred_at"] or "", m["created_at"])):
        if mark["role"] != COLLATERAL_LOCK:
            continue
        asset = mark["asset"]
        if releases_by_asset.get(asset, 0) > 0:
            releases_by_asset[asset] -= 1  # covered by a release
            continue
        open_locks.append(mark)
    return open_locks
