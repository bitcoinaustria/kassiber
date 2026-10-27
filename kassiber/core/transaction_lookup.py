"""Exact transaction references, with qualified txids for shared transactions.

A txid is not a row key: a CoinJoin, a batched deposit, or a self-transfer
between two owned wallets imports one row per wallet and direction under the
same txid. A reference therefore resolves in this order and fails closed:

1. the Kassiber transaction id, exactly;
2. the external id (txid, payment hash, exchange id), exactly, when it names
   one row;
3. a qualified reference ``<external id>:<qualifier>``, when the bare external
   id names several rows. The qualifier is a wallet id or label, ``in`` /
   ``out`` (``inbound`` / ``outbound``), or ``<wallet>:<in|out>``.

A reference that still names several rows raises ``ambiguous_reference``
with every candidate's full id, wallet, direction, amount and time, and a
remediation. Prefixes are never resolved.
"""

from __future__ import annotations

import sqlite3
from typing import Any, Mapping

from ..errors import AppError, build_remediation


DIRECTION_QUALIFIERS = {
    "in": "inbound",
    "inbound": "inbound",
    "out": "outbound",
    "outbound": "outbound",
}
MAX_REFERENCE_CANDIDATES = 20

_CANDIDATE_SQL = """
    SELECT t.*, w.label AS lookup_wallet_label
    FROM transactions t
    LEFT JOIN wallets w ON w.id = t.wallet_id
    WHERE t.profile_id = ? AND t.external_id = ?
    ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
"""


def reference_candidate(row: Mapping[str, Any]) -> dict[str, Any]:
    """A compact, privacy-safe description of one candidate row."""

    keys = row.keys() if hasattr(row, "keys") else ()
    amount = row["amount"]
    fee = row["fee"] if "fee" in keys else None
    return {
        "transaction_id": row["id"],
        "external_id": row["external_id"],
        "wallet_id": row["wallet_id"],
        "wallet": row["lookup_wallet_label"] if "lookup_wallet_label" in keys else None,
        "direction": row["direction"],
        "asset": row["asset"],
        "amount_msat": int(amount) if amount is not None else None,
        "fee_msat": int(fee) if fee is not None else None,
        "occurred_at": row["occurred_at"],
    }


def _external_rows(
    conn: sqlite3.Connection, profile_id: str, external_id: str
) -> list[sqlite3.Row]:
    return conn.execute(_CANDIDATE_SQL, (profile_id, external_id)).fetchall()


def _filter_direction(rows, direction):
    if direction is None:
        return list(rows)
    return [row for row in rows if row["direction"] == direction]


def _apply_qualifier(rows, qualifier: str):
    parts = qualifier.rsplit(":", 1)
    direction = None
    wallet = qualifier
    if qualifier.casefold() in DIRECTION_QUALIFIERS:
        direction, wallet = DIRECTION_QUALIFIERS[qualifier.casefold()], ""
    elif len(parts) == 2 and parts[1].casefold() in DIRECTION_QUALIFIERS:
        direction, wallet = DIRECTION_QUALIFIERS[parts[1].casefold()], parts[0]
    wallet = wallet.strip()
    selected = _filter_direction(rows, direction)
    if wallet:
        selected = [
            row
            for row in selected
            if row["wallet_id"] == wallet
            or str(row["lookup_wallet_label"] or "").casefold() == wallet.casefold()
        ]
    return selected


def ambiguous_reference_error(
    reference: str,
    rows,
    *,
    message: str | None = None,
) -> AppError:
    candidates = [reference_candidate(row) for row in rows[:MAX_REFERENCE_CANDIDATES]]
    return AppError(
        message or f"Transaction reference '{reference}' matches {len(rows)} transactions",
        code="ambiguous_reference",
        hint=(
            "Several rows share this reference, which is normal for CoinJoins and "
            "batched deposits. Use the full transaction id of the intended row, or "
            "qualify the reference as '<txid>:<wallet>' and/or ':in' / ':out'."
        ),
        details={
            "reference": reference,
            "candidate_count": len(rows),
            "candidates": candidates,
            "candidates_truncated": len(rows) > len(candidates),
            "remediation": build_remediation(
                "retry_with_full_transaction_id",
                target={
                    "transaction_ids": [item["transaction_id"] for item in candidates]
                },
                arguments_hint={
                    "transaction_id": "<one candidate's transaction_id>",
                    "qualified_reference": f"{reference}:<wallet id or label>:<in|out>",
                },
                message=(
                    "Retry the same call with one candidate's full transaction_id. "
                    "Do not choose between candidates yourself when the user's "
                    "intent does not single one out; ask."
                ),
            ),
        },
    )


def _plain_row(conn: sqlite3.Connection, row: sqlite3.Row) -> sqlite3.Row:
    # Callers get the same columns as an exact-id lookup, never the joined
    # lookup label.
    return conn.execute(
        "SELECT * FROM transactions WHERE id = ?", (row["id"],)
    ).fetchone()


def find_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
    reference: Any,
    *,
    direction: str | None = None,
) -> sqlite3.Row | None:
    """Resolve one transaction row, ``None`` when nothing matches.

    Raises ``ambiguous_reference`` (with candidates and remediation) instead
    of picking one of several rows.
    """

    text = str(reference or "").strip()
    if not text:
        return None
    query = "SELECT * FROM transactions WHERE profile_id = ? AND id = ?"
    params: list[Any] = [profile_id, text]
    if direction is not None:
        query += " AND direction = ?"
        params.append(direction)
    row = conn.execute(query, params).fetchone()
    if row is not None:
        return row

    rows = _filter_direction(_external_rows(conn, profile_id, text), direction)
    if len(rows) == 1:
        return _plain_row(conn, rows[0])
    if len(rows) > 1:
        direction_hint = f" {direction}" if direction else ""
        raise ambiguous_reference_error(
            text,
            rows,
            message=(
                f"Transaction external_id '{text}' matches multiple"
                f"{direction_hint} transactions"
            ),
        )

    # Only a reference with no exact match may be read as qualified, so an
    # external id that itself contains ':' keeps its exact meaning.
    for index, char in enumerate(text):
        if char != ":":
            continue
        base, qualifier = text[:index].strip(), text[index + 1 :].strip()
        if not base or not qualifier:
            continue
        shared = _filter_direction(_external_rows(conn, profile_id, base), direction)
        if not shared:
            continue
        selected = _apply_qualifier(shared, qualifier)
        if len(selected) == 1:
            return _plain_row(conn, selected[0])
        if len(selected) > 1:
            raise ambiguous_reference_error(text, selected)
        raise AppError(
            f"Qualifier '{qualifier}' matches none of the {len(shared)} "
            f"transactions with external_id '{base}'",
            code="reference_qualifier_unmatched",
            details={
                "reference": text,
                "candidate_count": len(shared),
                "candidates": [
                    reference_candidate(item)
                    for item in shared[:MAX_REFERENCE_CANDIDATES]
                ],
            },
        )
    return None


__all__ = [
    "DIRECTION_QUALIFIERS",
    "MAX_REFERENCE_CANDIDATES",
    "ambiguous_reference_error",
    "find_transaction",
    "reference_candidate",
]
