"""Self-transfer detection for cross-wallet on-chain hops.

When the same on-chain transaction appears as outbound in one kassiber wallet
and inbound in another wallet of the same profile, it is a self-transfer:
the user moved their own coins between their own wallets. RP2 models this as
an `IntraTransaction` (MOVE), where the network fee is the only taxable
portion and the lots themselves carry their original cost basis across to
the destination wallet.

This module is the pure detection layer. Conversion of detected pairs into
RP2 `IntraTransaction` instances happens in the journal pipeline.

Detection rule (intentionally conservative):

    Two rows share `(external_id, asset)`, sit in different wallets,
    and form exactly one outbound + one inbound. Multi-output transactions
    that fan out to several owned wallets are skipped — they would need
    explicit user disambiguation.
"""

from collections import defaultdict


def apply_manual_pairs(rows, auto_pairs, manual_pair_records):
    """Merge manual pair records with auto-detected pairs.

    Manual pairs (created via ``kassiber transfers pair``) take precedence
    over auto-detection: any auto-pair that touches a manually-paired
    transaction is dropped, and the manual pair takes its place.

    Same-asset manual pairs with ``policy=carrying-value`` feed back into
    the IntraTransaction pipeline (same shape as auto pairs). Cross-asset
    pairs are returned separately so the journal pipeline can record them
    as audit metadata without handing them to RP2. Any existing manual
    pair with a different policy still suppresses auto-detection for those
    rows, but the legs are left on the normal SELL + BUY path.

    Args:
        rows: full row list for the profile (sqlite3.Row-like).
        auto_pairs: output of ``detect_intra_transfers(rows)``.
        manual_pair_records: iterable of dicts with at least ``out_transaction_id``,
            ``in_transaction_id``, ``policy``, ``kind``.

    Returns:
        merged_pairs: list of ``{"out": out_row, "in": in_row}`` for
            same-asset pairs (manual + surviving auto), suitable for the
            existing intra path.
        cross_asset_pairs: list of dicts describing cross-asset manual
            pairs for audit purposes only.
    """
    rows_by_id = {row["id"]: row for row in rows}
    manual_same_asset = []
    cross_asset_pairs = []
    manually_paired_ids = set()
    for record in manual_pair_records:
        out_id = record["out_transaction_id"]
        in_id = record["in_transaction_id"]
        out_row = rows_by_id.get(out_id)
        in_row = rows_by_id.get(in_id)
        if out_row is None or in_row is None:
            continue
        manually_paired_ids.add(out_id)
        manually_paired_ids.add(in_id)
        if out_row["asset"] == in_row["asset"] and record["policy"] == "carrying-value":
            manual_same_asset.append({"out": out_row, "in": in_row})
        elif out_row["asset"] != in_row["asset"]:
            cross_asset_pairs.append(
                {
                    "pair_id": record["id"],
                    "kind": record["kind"],
                    "policy": record["policy"],
                    "out_id": out_id,
                    "in_id": in_id,
                    "out_asset": out_row["asset"],
                    "in_asset": in_row["asset"],
                }
            )
    surviving_auto = [
        pair
        for pair in auto_pairs
        if pair["out"]["id"] not in manually_paired_ids
        and pair["in"]["id"] not in manually_paired_ids
    ]
    return manual_same_asset + surviving_auto, cross_asset_pairs


def detect_intra_transfers(rows):
    """Return ``(pairs, matched_ids)`` for the given transaction rows.

    Args:
        rows: iterable of sqlite3.Row-like records that expose
            ``id``, ``external_id``, ``asset``, ``direction``, ``amount``,
            ``wallet_id``.

    Returns:
        pairs: list of ``{"out": out_row, "in": in_row}`` dicts.
        matched_ids: set of transaction ids covered by any pair.
    """
    by_key = defaultdict(list)
    for row in rows:
        external_id = row["external_id"] if "external_id" in row.keys() else None
        if not external_id:
            continue
        by_key[(external_id, row["asset"])].append(row)

    pairs = []
    matched_ids = set()
    for group in by_key.values():
        outs = [
            r
            for r in group
            if r["direction"] == "outbound" and (r["amount"] or 0) > 0
        ]
        ins = [r for r in group if r["direction"] == "inbound"]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_row, in_row = outs[0], ins[0]
        if out_row["wallet_id"] == in_row["wallet_id"]:
            continue
        pairs.append({"out": out_row, "in": in_row})
        matched_ids.add(out_row["id"])
        matched_ids.add(in_row["id"])
    return pairs, matched_ids
