"""Collateral marks: tag a transaction as Bitcoin-backed-loan collateral.

A *collateral lock* (an outbound posting BTC as loan collateral) is not a
disposal — the borrower still owns the coins, just encumbered. The matching
*collateral release* (the inbound when the collateral returns on repayment) is
not an acquisition: the coins re-enter the pool they never really left, so a
lock/release round-trip nets to zero and preserves the original basis and
acquisition date (Alt/Neu by date, not a hold period).

Deliberately minimal. A mark is one row in ``loan_legs`` linking a journal
transaction to a role — there is no facility record, and no custody,
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

# The two collateral roles. A lock suppresses the outbound disposal; a release
# suppresses the inbound acquisition. The strings are also the membership values
# the tax engine checks, so keep them stable.
COLLATERAL_LOCK = "collateral_lock"
COLLATERAL_RELEASE = "collateral_release"
COLLATERAL_ROLES = (COLLATERAL_LOCK, COLLATERAL_RELEASE)

# Outbound leg whose coins stay owned (encumbered): suppress the disposal.
LOCK_SUPPRESS_ROLES = frozenset({COLLATERAL_LOCK})
# Inbound leg whose coins return to the pool they never left: suppress the
# acquisition so a lock/release round-trip nets to nothing.
RELEASE_SUPPRESS_ROLES = frozenset({COLLATERAL_RELEASE})

# Human labels for the CLI / GUI / reconcile hints.
ROLE_LABELS = {
    COLLATERAL_LOCK: "loan collateral (out)",
    COLLATERAL_RELEASE: "collateral returned (in)",
}


def _require_role(role: str) -> None:
    if role not in COLLATERAL_ROLES:
        raise AppError(
            f"Invalid collateral role '{role}'. Allowed: {', '.join(COLLATERAL_ROLES)}",
            code="validation",
            details={"field": "role", "value": role},
        )


def mark_collateral(
    conn,
    workspace_id: str,
    profile_id: str,
    transaction_id: str,
    *,
    role: str,
    note: Optional[str] = None,
    commit: bool = True,
) -> dict[str, Any]:
    """Mark a transaction as a collateral lock or release. One active mark per
    transaction — re-marking replaces the existing role."""
    _require_role(role)
    tx = conn.execute(
        "SELECT id FROM transactions WHERE id = ? AND profile_id = ?",
        (transaction_id, profile_id),
    ).fetchone()
    if tx is None:
        raise AppError(
            f"Transaction '{transaction_id}' not found in this profile",
            code="not_found",
            details={"transaction_id": transaction_id},
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
        INSERT INTO loan_legs(id, workspace_id, profile_id, transaction_id, role, note, deleted_at, created_at)
        VALUES(?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (leg_id, workspace_id, profile_id, transaction_id, role, note, now_iso()),
    )
    if commit:
        conn.commit()
    row = conn.execute("SELECT * FROM loan_legs WHERE id = ?", (leg_id,)).fetchone()
    return {key: row[key] for key in row.keys()}


def unmark_collateral(
    conn, profile_id: str, transaction_id: str, *, commit: bool = True
) -> dict[str, Any]:
    """Remove the collateral mark from a transaction. The transaction reverts to
    its normal tax classification — a removed lock books as the disposal it is."""
    row = conn.execute(
        "SELECT id FROM loan_legs WHERE profile_id = ? AND transaction_id = ? AND deleted_at IS NULL",
        (profile_id, transaction_id),
    ).fetchone()
    if row is None:
        raise AppError(
            f"No collateral mark on transaction '{transaction_id}'",
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


def load_collateral_role_map(conn, profile_id: str) -> dict[str, str]:
    """``{transaction_id: role}`` for active collateral marks — consumed by the
    tax pipeline to classify the matching journal transaction by its role."""
    rows = conn.execute(
        "SELECT transaction_id, role FROM loan_legs WHERE profile_id = ? AND deleted_at IS NULL",
        (profile_id,),
    ).fetchall()
    return {str(row["transaction_id"]): str(row["role"]) for row in rows}


def list_collateral_marks(conn, profile_id: str) -> list[dict[str, Any]]:
    """All active collateral marks joined to their transaction, newest first."""
    rows = conn.execute(
        """
        SELECT ll.transaction_id, ll.role, ll.note, ll.created_at,
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
