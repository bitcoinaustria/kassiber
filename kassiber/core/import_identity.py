"""Read-only compatibility for positional CSV identities and duplicate review."""

from collections import Counter, defaultdict
import json

from ..importers import synthetic_csv_identity


DUPLICATE_REVIEW = "synthetic_import_duplicate_review_required"


def stored_synthetic_occurrences(conn, *, profile_id, wallet_id=None):
    """Number stored occurrences by import time, then the old numeric row index.

    Excluded rows still consume an occurrence: re-import cannot reopen a review.
    No ID, raw row, fingerprint, review, or batch membership is rewritten.
    """
    rows = conn.execute("""
        SELECT t.id, t.wallet_id, t.external_id, t.raw_json, t.excluded,
               t.workspace_id, t.profile_id, t.created_at,
               b.id AS batch_id, b.imported_at
        FROM transactions t
        LEFT JOIN import_batch_transactions bt ON bt.transaction_id = t.id
        LEFT JOIN import_batches b ON b.id = bt.batch_id
        WHERE t.profile_id = ?
          AND (? IS NULL OR t.wallet_id = ?)
          AND (t.external_id LIKE 'ledgerlive:%'
            OR t.external_id LIKE 'coinfinity:coinfinity:%'
            OR t.external_id LIKE 'pocketbitcoin:pocketbitcoin:%'
            OR t.external_id LIKE 'binance-supplemental:%'
            OR t.external_id LIKE 'binance:%')
        ORDER BY COALESCE(b.imported_at, t.created_at), b.id, t.id
    """, (profile_id, wallet_id, wallet_id)).fetchall()
    groups = defaultdict(list)
    seen = set()
    for row in rows:
        if row["id"] in seen:
            continue
        seen.add(row["id"])
        identity = synthetic_csv_identity(row["external_id"], row["raw_json"])
        if identity is None:
            continue
        economic, ordinal, legacy = identity
        groups[(row["wallet_id"], economic)].append({
            **dict(row), "ordinal": ordinal, "legacy": legacy,
        })
    for members in groups.values():
        members.sort(key=lambda row: (row["imported_at"] or row["created_at"],
                                      row["ordinal"], row["id"]))
    return dict(groups)


def duplicate_review_quarantines(conn, profile_id):
    """Hold every active member of a cross-batch excess; choose no survivor."""
    quarantines = []
    for (_, economic), members in stored_synthetic_occurrences(conn, profile_id=profile_id).items():
        active = [row for row in members if not row["excluded"]]
        batches = {row["batch_id"] for row in active if row["batch_id"] is not None}
        if len(batches) < 2:
            continue
        counts = Counter(row["batch_id"] for row in members)
        # A current occurrence suffix proves that many rows appeared in its
        # source export, even when the batch created only the new remainder.
        for row in members:
            if not row["legacy"]:
                counts[row["batch_id"]] = max(counts[row["batch_id"]], row["ordinal"])
        largest = max(counts.values())
        if len(active) <= largest:
            continue
        detail = json.dumps({
            "economic_hash": economic, "transaction_ids": sorted(row["id"] for row in active),
            "active_count": len(active), "largest_single_batch_count": largest,
            "import_batch_count": len(batches),
            "resolution": "Review the source exports and use metadata exclude --transaction <id> for the duplicate rows, then rebuild journals. No survivor is selected automatically.",
        }, sort_keys=True)
        quarantines.extend({
            "transaction_id": row["id"], "workspace_id": row["workspace_id"],
            "profile_id": row["profile_id"], "reason": DUPLICATE_REVIEW, "detail_json": detail,
        } for row in active)
    return tuple(quarantines)
