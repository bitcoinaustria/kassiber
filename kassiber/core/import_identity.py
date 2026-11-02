"""Compatibility for positional CSV identities and their duplicate review.

Import matching and detection are read-only; only an explicit owner review
writes, appending a confirmation that changes no transaction.
"""

from collections import Counter, defaultdict
import hashlib
import json
import uuid

from ..errors import AppError
from ..importers import synthetic_csv_identity
from ..time_utils import now_iso


DUPLICATE_REVIEW = "synthetic_import_duplicate_review_required"
CONFIRM_DISTINCT = "confirm_distinct_import_rows"
RESOLUTION = (
    "Review the source exports. If some rows are copies of one trade, use metadata exclude "
    "--transaction <id> for the duplicate rows. If every row is a genuine separate trade, apply "
    "the confirm_distinct_import_rows review with this group_fingerprint. Then rebuild journals. "
    "No survivor is selected automatically."
)


def stored_synthetic_occurrences(conn, *, profile_id, wallet_id=None):
    """Group stored occurrences by import time, then the old numeric row index.

    Rows without a batch predate batch recording, so they sort before every
    batched row. Excluded rows still consume an occurrence: re-import cannot
    reopen a review. No ID, raw row, fingerprint, review, or batch membership
    is rewritten.
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
        ORDER BY b.id IS NOT NULL, COALESCE(b.imported_at, t.created_at), b.id, t.id
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
        members.sort(key=lambda row: (row["batch_id"] is not None,
                                      row["imported_at"] or row["created_at"],
                                      row["ordinal"], row["id"]))
    return dict(groups)


def occurrence_slots(members):
    """Map each occurrence number to the stored row that already holds it.

    A current ID names its own occurrence exactly. Old positional rows carry a
    file position instead, so they fill only the lowest numbers no current row
    holds, in stored order.
    """
    slots = {}
    for row in members:
        if not row["legacy"]:
            slots.setdefault(row["ordinal"], row)
    free = 0
    for row in members:
        if row["legacy"]:
            free += 1
            while free in slots:
                free += 1
            slots[free] = row
    return slots


def group_fingerprint(wallet_id, economic, members):
    """Bind a review to the exact members of a group and their exclusion."""
    payload = json.dumps({
        "wallet_id": wallet_id, "economic_hash": economic,
        "members": sorted([row["id"], bool(row["excluded"])] for row in members),
    }, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode()).hexdigest()


def held_duplicate_groups(conn, profile_id):
    """Every cross-batch excess no owner has confirmed as distinct trades."""
    confirmed = {(row["wallet_id"], row["economic_hash"], row["group_fingerprint"]) for row in conn.execute(
        "SELECT wallet_id, economic_hash, group_fingerprint FROM synthetic_import_duplicate_reviews "
        "WHERE profile_id = ?", (profile_id,),
    )}
    groups = []
    for (wallet_id, economic), members in stored_synthetic_occurrences(conn, profile_id=profile_id).items():
        active = [row for row in members if not row["excluded"]]
        # Unbatched rows came from imports before batch recording began; they
        # count together as one more import beside any batched copy.
        batches = {row["batch_id"] for row in active}
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
        fingerprint = group_fingerprint(wallet_id, economic, members)
        if (wallet_id, economic, fingerprint) in confirmed:
            continue
        groups.append({
            "wallet_id": wallet_id, "economic_hash": economic, "group_fingerprint": fingerprint,
            "active": active, "largest": largest, "import_count": len(batches),
        })
    return groups


def duplicate_review_quarantines(conn, profile_id):
    """Hold every active member of a cross-batch excess; choose no survivor."""
    quarantines = []
    for group in held_duplicate_groups(conn, profile_id):
        active = group["active"]
        detail = json.dumps({
            "economic_hash": group["economic_hash"], "group_fingerprint": group["group_fingerprint"],
            "transaction_ids": sorted(row["id"] for row in active),
            "active_count": len(active), "largest_single_batch_count": group["largest"],
            "import_batch_count": group["import_count"],
            "resolution": RESOLUTION,
        }, sort_keys=True)
        quarantines.extend({
            "transaction_id": row["id"], "workspace_id": row["workspace_id"],
            "profile_id": row["profile_id"], "reason": DUPLICATE_REVIEW, "detail_json": detail,
        } for row in active)
    return tuple(quarantines)


def confirm_distinct(conn, profile, *, transaction_id, expected_fingerprint, reason, authored_source):
    """Record the owner's review that a held group holds only distinct trades.

    Bound to the exact group it confirms. Nothing is excluded or rewritten.
    """
    group = next((group for group in held_duplicate_groups(conn, profile["id"])
                  if transaction_id in {row["id"] for row in group["active"]}), None)
    if group is None or group["group_fingerprint"] != expected_fingerprint:
        raise AppError("Import duplicate review changed; inspect it again", code="review_case_changed",
                       retryable=False)
    transaction_ids = sorted(row["id"] for row in group["active"])
    review_id = str(uuid.uuid4())
    conn.execute(
        """INSERT INTO synthetic_import_duplicate_reviews(id, workspace_id, profile_id, wallet_id,
               economic_hash, group_fingerprint, transaction_ids_json, reason, authored_source, created_at)
           VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (review_id, profile["workspace_id"], profile["id"], group["wallet_id"], group["economic_hash"],
         expected_fingerprint, json.dumps(transaction_ids), reason, authored_source, now_iso()),
    )
    conn.execute(
        """UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0,
               journal_input_version = journal_input_version + 1, ownership_review_counts_json = NULL
           WHERE id = ?""", (profile["id"],),
    )
    return {"review_id": review_id, "group_fingerprint": expected_fingerprint,
            "transaction_ids": transaction_ids}
