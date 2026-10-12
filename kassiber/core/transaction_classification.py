"""Booked meaning of transactions, shared by exports.

The year handoff and the tax-platform exports describe each transaction the
same way: a classification read from the processed journal, the pair links of
booked moves and reviewed swaps, and the quarantine barrier. Keeping these
here means two exports of one book cannot disagree about what a row was.
Nothing here computes custody or tax; it only reads the journal projection.
"""

from __future__ import annotations

from collections import defaultdict
from typing import Any, Callable

from ..errors import AppError

# Order in which classifications are joined when one transaction carries
# several kinds of journal rows (e.g. a payment that also moves change to
# another own wallet).
CLASSIFICATION_ORDER = (
    "internal_transfer",
    "swap",
    "income",
    "acquisition",
    "disposal",
    "fee",
)
_PAIR_REF_LENGTH = 12


def classification_entry_type(entry_type, at_category) -> str:
    """The entry type as classification reads it.

    A disposal booked tax-neutral (Austrian ``neu_swap``) moves basis; it
    is not a sale. Keeping it apart lets a transaction that also realizes a
    gain (a swap payout settled partly as a taxable disposal) keep both.
    """
    if entry_type == "disposal" and at_category == "neu_swap":
        return "neutral_disposal"
    return str(entry_type)


def classify(entry_types, at_categories, is_swap) -> str:
    labels = set()
    if entry_types & {"transfer_in", "transfer_out", "transfer_fee"}:
        labels.add("internal_transfer")
    if is_swap or "neu_swap" in at_categories:
        labels.add("swap")
    if "income" in entry_types:
        labels.add("income")
    elif "acquisition" in entry_types and not is_swap:
        labels.add("acquisition")
    # ``disposal`` here is realized; tax-neutral ones arrive as
    # ``neutral_disposal`` (see ``classification_entry_type``).
    if "disposal" in entry_types:
        labels.add("disposal")
    if not labels and "fee" in entry_types:
        labels.add("fee")
    if not labels:
        return "no_journal_entry"
    return "+".join(label for label in CLASSIFICATION_ORDER if label in labels)


def journal_entry_types(conn, profile_id) -> dict[str, dict[str, set[str]]]:
    """Entry types and tax categories per transaction across all periods."""
    facts: dict[str, dict[str, set[str]]] = defaultdict(
        lambda: {"entry_types": set(), "at_categories": set()}
    )
    for row in conn.execute(
        "SELECT transaction_id, entry_type, at_category FROM journal_entries WHERE profile_id = ?",
        (profile_id,),
    ).fetchall():
        fact = facts[row["transaction_id"]]
        fact["entry_types"].add(classification_entry_type(row["entry_type"], row["at_category"]))
        if row["at_category"]:
            fact["at_categories"].add(str(row["at_category"]))
    return facts


def pair_links(conn, profile_id, external_ids):
    """Pair refs and the other leg's transaction id for every paired leg.

    A pair is one booked custody move, reviewed conversion, or direct swap
    payout. Both legs carry the same short ref; a leg that takes part in
    several pairs (consolidation, fan-out, split swap) lists each of them.
    Returns ``(links, swap_transaction_ids)``. A swap is a conversion the
    processed journal books as tax-neutral (Austrian ``neu_swap``). What the
    journal applied decides, not the policy requested at pairing: a carrying
    conversion of Altbestand still realizes a gain. Every other conversion is
    a disposal and an acquisition and keeps that classification; see
    ``conversion_leg_ids``.
    """
    links: dict[str, dict[str, set[str]]] = defaultdict(lambda: {"refs": set(), "others": set()})
    swap_ids: set[str] = set()
    tax_neutral = _tax_neutral_swap_ids(conn, profile_id)
    for relation in conn.execute(
        """
        SELECT id, relation_kind, out_transaction_id, in_transaction_id,
               target_external_id
        FROM journal_custody_projection_relations
        WHERE profile_id = ?
          AND relation_kind IN ('move', 'conversion', 'direct_payout')
        """,
        (profile_id,),
    ).fetchall():
        ref = str(relation["id"])[:_PAIR_REF_LENGTH]
        out_id = relation["out_transaction_id"]
        in_id = relation["in_transaction_id"]
        if relation["relation_kind"] != "move" and (out_id in tax_neutral or in_id in tax_neutral):
            swap_ids.update(value for value in (out_id, in_id) if value)
        if out_id:
            links[out_id]["refs"].add(ref)
            other = (
                external_ids.get(in_id) or in_id
                if in_id
                else relation["target_external_id"] or ""
            )
            if other:
                links[out_id]["others"].add(other)
        if in_id:
            links[in_id]["refs"].add(ref)
            if out_id:
                links[in_id]["others"].add(external_ids.get(out_id) or out_id)
    return links, swap_ids


def _tax_neutral_swap_ids(conn, profile_id) -> set[str]:
    return {
        row["transaction_id"]
        for row in conn.execute(
            """
            SELECT DISTINCT transaction_id FROM journal_entries
            WHERE profile_id = ? AND at_category = 'neu_swap'
            """,
            (profile_id,),
        ).fetchall()
    }


def conversion_leg_ids(conn, profile_id) -> set[str]:
    """Both legs of every reviewed conversion or direct swap payout."""
    ids: set[str] = set()
    for relation in conn.execute(
        """
        SELECT out_transaction_id, in_transaction_id
        FROM journal_custody_projection_relations
        WHERE profile_id = ? AND relation_kind IN ('conversion', 'direct_payout')
        """,
        (profile_id,),
    ).fetchall():
        ids.update(value for value in (relation["out_transaction_id"], relation["in_transaction_id"]) if value)
    return ids


def require_no_quarantines(
    conn,
    profile_id,
    *,
    in_scope: Callable[[Any], bool],
    code: str,
    message: str,
    hint: str,
    details: dict[str, Any] | None = None,
) -> None:
    """Fail closed when a quarantined transaction falls inside an export.

    A quarantined row has no journal entry, so an export would either drop it
    or guess its meaning. ``in_scope`` receives each quarantine row (with
    ``occurred_at`` and ``wallet_id``). The error lists up to 20 of them.
    """
    rows = conn.execute(
        """
        SELECT jq.transaction_id, jq.reason, t.occurred_at, t.wallet_id,
               COALESCE(t.external_id, '') AS external_id,
               w.label AS wallet
        FROM journal_quarantines jq
        JOIN transactions t ON t.id = jq.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE jq.profile_id = ?
        ORDER BY t.occurred_at ASC, jq.transaction_id ASC
        """,
        (profile_id,),
    ).fetchall()
    blocking = [row for row in rows if in_scope(row)]
    if not blocking:
        return
    raise AppError(
        message,
        code=code,
        hint=hint,
        details={
            **(details or {}),
            "count": len(blocking),
            "transactions": [
                {
                    "transaction_ref": row["transaction_id"],
                    "transaction_id": row["external_id"],
                    "wallet": row["wallet"],
                    "occurred_at": row["occurred_at"],
                    "reason": row["reason"],
                }
                for row in blocking[:20]
            ],
        },
        retryable=False,
    )


__all__ = [
    "CLASSIFICATION_ORDER",
    "classify",
    "journal_entry_types",
    "pair_links",
    "require_no_quarantines",
]
