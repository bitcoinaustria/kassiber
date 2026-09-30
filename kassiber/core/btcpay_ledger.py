"""Wallet rows Kassiber derives from BTCPay's own accounting.

Two seams live here, both reading provenance the caller already refreshed
(no network I/O):

- Payout fees. BTCPay's wallet history reports a send as one net amount with
  the miner fee folded in. When completed payouts explain a send, the rest of
  it is the fee, so the row books principal and fee separately instead of
  treating the fee as part of the disposal.
- Payment ledgers. Lightning nodes, plugin rails, and layer-two wallets that
  BTCPay settles into often cannot be watched by Kassiber. A payment ledger
  books BTCPay's settled payments as receipts and its completed payouts as
  sends, so the balance comes from BTCPay's records. It sees only what passes
  through BTCPay; funds moved outside it need their own record.
"""

from __future__ import annotations

import json
import sqlite3
from decimal import Decimal
from typing import Any, Iterable, Mapping, Sequence

from ..btcpay.payment_methods import canonical_payment_method_id, ledger_group
from ..fingerprints import make_transaction_fingerprint
from ..msat import btc_to_msat, msat_to_btc

SOURCE_MODE_CONFIG_KEY = "source_mode"
LEDGER_SOURCE_MODE = "payments"
LEDGER_PAYMENT_METHODS_CONFIG_KEY = "payment_method_ids"
LEDGER_PAYMENT_HASH_SOURCE = "btcpay"
LEDGER_RAW_SOURCE = "btcpay_payment_ledger"

# A remainder above this is more likely an output Kassiber does not know about
# (for example a manual send batched with payouts) than a miner fee.
MAX_DERIVED_FEE_MSAT = 200_000_000
SETTLED_PAYMENT_STATUSES = frozenset({"settled"})
COMPLETED_PAYOUT_STATUSES = frozenset({"completed"})
_SQL_CHUNK = 400


# ---------------------------------------------------------------------------
# Payout fees
# ---------------------------------------------------------------------------


def payout_totals_by_txid(conn: sqlite3.Connection, profile_id: str, *, asset: str) -> dict[str, dict[str, int]]:
    """Broadcast payouts per proving txid: summed amount and payout count.

    In-progress payouts count too: BTCPay records the txid when it broadcasts
    a batch, and payouts of one send can complete at different times.
    """

    rows = conn.execute(
        """
        SELECT txid, amount
        FROM btcpay_provenance_records
        WHERE profile_id = ?
          AND record_type = 'payout'
          AND txid IS NOT NULL
          AND amount IS NOT NULL
          AND asset = ?
          AND LOWER(COALESCE(status, '')) IN ('inprogress', 'completed')
        """,
        (profile_id, asset),
    ).fetchall()
    totals: dict[str, dict[str, int]] = {}
    for row in rows:
        entry = totals.setdefault(str(row["txid"]).lower(), {"amount_msat": 0, "payouts": 0})
        entry["amount_msat"] += abs(int(row["amount"]))
        entry["payouts"] += 1
    return totals


def derived_payout_fee_msat(net_msat: int, payout_msat: int) -> int | None:
    """The miner fee inside a send whose payouts are known, or None if unclear."""

    if payout_msat <= 0 or net_msat <= payout_msat:
        return None
    fee = net_msat - payout_msat
    if fee > MAX_DERIVED_FEE_MSAT or fee * 2 >= net_msat:
        return None
    return fee


def outbound_fees_by_txid(conn: sqlite3.Connection, profile_id: str, wallet_id: str) -> dict[str, int]:
    """Stored fee of each outbound row in a wallet, keyed by lower-case txid."""

    return {
        str(row["external_id"]).lower(): int(row["fee"] or 0)
        for row in conn.execute(
            """
            SELECT external_id, fee FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND direction = 'outbound' AND external_id IS NOT NULL
            """,
            (profile_id, wallet_id),
        ).fetchall()
    }


def apply_payout_fees(
    records: Iterable[dict[str, Any]],
    totals: Mapping[str, Mapping[str, int]],
    stored_fees: Mapping[str, int] | None = None,
) -> int:
    """Set ``fee`` on fee-inclusive outbound wallet records BTCPay payouts explain.

    A send already in the book keeps its stored fee so the re-fetched record
    lands on the same row; only ``backfill_payout_fees`` changes stored rows.
    """

    applied = 0
    stored_fees = stored_fees or {}
    for record in records:
        if record.get("direction") != "outbound" or not record.get("amount_includes_fee"):
            continue
        txid = str(record.get("txid") or "").lower()
        if txid in stored_fees:
            if stored_fees[txid]:
                record["fee"] = msat_to_btc(stored_fees[txid])
            continue
        total = totals.get(txid)
        if not total:
            continue
        fee = derived_payout_fee_msat(btc_to_msat(record.get("amount")) or 0, total["amount_msat"])
        if fee is None:
            continue
        record["fee"] = msat_to_btc(fee)
        applied += 1
    return applied


def backfill_payout_fees(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    totals: Mapping[str, Mapping[str, int]],
) -> int:
    """Split the fee out of already-imported sends once their payouts complete.

    Runs before the import so a re-fetched send lands on the same row: the
    fingerprint is recomputed with the new fee exactly as the import would.
    """

    if not totals:
        return 0
    rows = conn.execute(
        """
        SELECT id, external_id, occurred_at, direction, asset, amount
        FROM transactions
        WHERE profile_id = ? AND wallet_id = ?
          AND direction = 'outbound'
          AND amount_includes_fee = 1
          AND COALESCE(fee, 0) = 0
          AND external_id IS NOT NULL
        """,
        (profile_id, wallet_id),
    ).fetchall()
    updated = 0
    for row in rows:
        total = totals.get(str(row["external_id"]).lower())
        if not total:
            continue
        fee = derived_payout_fee_msat(int(row["amount"]), total["amount_msat"])
        if fee is None:
            continue
        fingerprint = make_transaction_fingerprint(
            wallet_id,
            row["external_id"],
            row["occurred_at"],
            row["direction"],
            row["asset"],
            msat_to_btc(row["amount"]),
            msat_to_btc(fee),
        )
        taken = conn.execute(
            "SELECT 1 FROM transactions WHERE fingerprint = ? AND id != ?",
            (fingerprint, row["id"]),
        ).fetchone()
        if taken:
            continue
        conn.execute(
            "UPDATE transactions SET fee = ?, fingerprint = ? WHERE id = ?",
            (fee, fingerprint, row["id"]),
        )
        updated += 1
    return updated


# ---------------------------------------------------------------------------
# Payment ledgers
# ---------------------------------------------------------------------------


def is_payment_ledger_config(config: Mapping[str, Any] | None) -> bool:
    return isinstance(config, Mapping) and str(config.get(SOURCE_MODE_CONFIG_KEY) or "").strip().lower() == LEDGER_SOURCE_MODE


def ledger_payment_method_ids(config: Mapping[str, Any]) -> list[str]:
    raw = config.get(LEDGER_PAYMENT_METHODS_CONFIG_KEY)
    values = raw if isinstance(raw, list) else [config.get("payment_method_id")]
    methods = []
    for value in values:
        method = canonical_payment_method_id(value)
        if method and ledger_group(method) and method not in methods:
            methods.append(method)
    return methods


def ledger_groups(payment_method_ids: Iterable[str]) -> set[str]:
    return {group for group in (ledger_group(method) for method in payment_method_ids) if group}


def _ledger_description(row: sqlite3.Row) -> str:
    if row["record_type"] == "payout":
        if row["origin_kind"] == "refund":
            return f"BTCPay refund for invoice {row['invoice_id']}" if row["invoice_id"] else "BTCPay refund"
        return f"BTCPay payout {row['origin_label']}".strip() if row["origin_label"] else "BTCPay payout"
    label = row["origin_label"] or row["order_id"] or row["invoice_id"]
    return f"BTCPay payment · {label}" if label else "BTCPay payment"


def ledger_records(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    store_id: str,
    payment_method_ids: Sequence[str],
) -> tuple[list[dict[str, Any]], dict[str, int]]:
    """Import records for a store's payment ledger, built from stored provenance.

    Each record's identity is the provenance ``stable_key``, so reruns update
    the same row and the commercial matcher links it exactly. Payment hashes
    are kept for cross-wallet duplicate checks but never as Lightning-node
    evidence; the source label keeps them out of automatic transfer pairing.
    """

    groups = ledger_groups(payment_method_ids)
    counts = {"payments": 0, "payouts": 0, "pending": 0}
    if not groups:
        return [], counts
    rows = conn.execute(
        """
        SELECT *
        FROM btcpay_provenance_records
        WHERE profile_id = ? AND store_id = ?
          AND record_type IN ('payment', 'payout')
          AND amount IS NOT NULL AND amount > 0
          AND occurred_at IS NOT NULL
        ORDER BY occurred_at ASC, stable_key ASC
        """,
        (profile_id, store_id),
    ).fetchall()
    records: list[dict[str, Any]] = []
    seen = set()
    for row in rows:
        if ledger_group(row["payment_method_id"]) not in groups or row["stable_key"] in seen:
            continue
        status = str(row["status"] or "").strip().lower()
        payout = row["record_type"] == "payout"
        if status not in (COMPLETED_PAYOUT_STATUSES if payout else SETTLED_PAYMENT_STATUSES):
            counts["pending"] += 1
            continue
        seen.add(row["stable_key"])
        counts["payouts" if payout else "payments"] += 1
        records.append(
            {
                "id": row["stable_key"],
                "occurred_at": row["occurred_at"],
                "confirmed_at": row["occurred_at"],
                "direction": "outbound" if payout else "inbound",
                "asset": row["asset"] or "BTC",
                "amount": msat_to_btc(abs(int(row["amount"]))),
                "fee": Decimal("0"),
                "amount_includes_fee": False,
                "kind": "withdrawal" if payout else "deposit",
                "description": _ledger_description(row),
                "counterparty": None,
                "payment_hash": row["payment_hash"],
                "payment_hash_source": LEDGER_PAYMENT_HASH_SOURCE if row["payment_hash"] else None,
                "raw_json": json.dumps(
                    {
                        "source": LEDGER_RAW_SOURCE,
                        "stable_key": row["stable_key"],
                        "store_id": row["store_id"],
                        "payment_method_id": row["payment_method_id"],
                        "record_type": row["record_type"],
                        "invoice_id": row["invoice_id"],
                    },
                    sort_keys=True,
                ),
            }
        )
    return records, counts


def _payment_hashes_elsewhere(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    keys: Iterable[tuple[str, str]],
) -> set[tuple[str, str]]:
    """``(payment_hash, direction)`` pairs another active wallet already books.

    Direction matters: the user's own wallet paying a store invoice books the
    same hash outbound, which is a transfer, not a duplicate.
    """

    found: set[tuple[str, str]] = set()
    unique = sorted({(value.lower(), direction) for value, direction in keys if value})
    hashes = sorted({value for value, _ in unique})
    for start in range(0, len(hashes), _SQL_CHUNK):
        chunk = hashes[start : start + _SQL_CHUNK]
        placeholders = ",".join("?" for _ in chunk)
        rows = conn.execute(
            f"""
            SELECT DISTINCT LOWER(payment_hash) AS payment_hash, direction FROM transactions
            WHERE profile_id = ? AND wallet_id != ? AND excluded = 0
              AND LOWER(payment_hash) IN ({placeholders})
            """,
            (profile_id, wallet_id, *chunk),
        ).fetchall()
        found.update((row["payment_hash"], row["direction"]) for row in rows)
    return found & set(unique)


def split_tracked_elsewhere(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    records: Sequence[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Hold back ledger rows another wallet already books by payment hash.

    A connected Lightning node records the same payments; importing them twice
    would double the store's balance and income.
    """

    def key(record: Mapping[str, Any]) -> tuple[str, str]:
        return str(record.get("payment_hash") or "").lower(), str(record.get("direction") or "")

    elsewhere = _payment_hashes_elsewhere(conn, profile_id, wallet_id, [key(record) for record in records])
    keep, held = [], []
    for record in records:
        (held if key(record) in elsewhere else keep).append(record)
    return keep, held


def existing_rows_tracked_elsewhere(conn: sqlite3.Connection, profile_id: str, wallet_id: str) -> dict[str, Any]:
    """Ledger rows that became duplicates after another wallet was connected.

    Returns the count and the labels of the wallets that book them too.
    """

    keys = [
        (row["payment_hash"], row["direction"])
        for row in conn.execute(
            """
            SELECT payment_hash, direction FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND payment_hash IS NOT NULL AND excluded = 0
            """,
            (profile_id, wallet_id),
        ).fetchall()
    ]
    duplicates = _payment_hashes_elsewhere(conn, profile_id, wallet_id, keys)
    wallets: list[str] = []
    hashes = sorted({payment_hash for payment_hash, _ in duplicates})
    for start in range(0, len(hashes), _SQL_CHUNK):
        chunk = hashes[start : start + _SQL_CHUNK]
        placeholders = ",".join("?" for _ in chunk)
        for row in conn.execute(
            f"""
            SELECT DISTINCT w.label FROM transactions t JOIN wallets w ON w.id = t.wallet_id
            WHERE t.profile_id = ? AND t.wallet_id != ? AND t.excluded = 0
              AND LOWER(t.payment_hash) IN ({placeholders})
            ORDER BY w.label
            """,
            (profile_id, wallet_id, *chunk),
        ).fetchall():
            if row["label"] not in wallets:
                wallets.append(row["label"])
    return {"count": len(duplicates), "wallets": wallets}


__all__ = [
    "LEDGER_PAYMENT_METHODS_CONFIG_KEY",
    "LEDGER_SOURCE_MODE",
    "SOURCE_MODE_CONFIG_KEY",
    "apply_payout_fees",
    "backfill_payout_fees",
    "derived_payout_fee_msat",
    "existing_rows_tracked_elsewhere",
    "outbound_fees_by_txid",
    "is_payment_ledger_config",
    "ledger_groups",
    "ledger_payment_method_ids",
    "ledger_records",
    "payout_totals_by_txid",
    "split_tracked_elsewhere",
]
