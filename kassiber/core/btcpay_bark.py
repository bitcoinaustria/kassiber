"""Book a store's Bark wallet from BTCPay without counting a payment twice.

Reads only what the caller already fetched and stored (no network I/O):

- Identities. A Bark row that pays a BTCPay invoice payment (by VTXO
  outpoint, Lightning payment hash, or a pre-0.10 ``#ark-movement-N`` id with
  its invoice label) or a payout (by payment hash or transaction) is booked
  under that provenance record's ``stable_key``: the identity a payment
  ledger booked it under, and the one the commercial matcher links exactly.
  Other rows use ``btcpay-bark:{store}:{row id}``. A stored identity sticks; it
  only moves from that fallback to a provenance key that appears later.
- Migration. A ``BTC-ARK`` payment ledger whose plugin now serves wallet
  history becomes a wallet import: every active ledger row must match exactly
  one Bark record of the same direction and principal, and is kept under its
  own id (notes, reviews, links) with Bark's fee. Anything else refuses before
  a write.
- Lightning overlap. Lightning payments that settle into the Bark wallet are
  the Bark import's: a ``BTC-LN`` payment-ledger row of the same store that
  books the same payment hash in the same direction moves into the Bark
  wallet. Another wallet booking it refuses.
- Network. No Bark field names it; the store's Bark payment destinations do
  (Ark addresses ``ark1`` on mainnet, ``tark1`` on every test network). A
  book bound to another network refuses the import.
"""

from __future__ import annotations

import json
import sqlite3
from typing import Any, Mapping, Sequence

from ..btcpay.bark import (
    BARK_PAYMENT_METHOD_ID,
    BARK_RAW_SOURCE,
    PART_PRINCIPAL,
    fallback_identity,
    is_fallback_identity,
    payment_keys,
)
from ..btcpay.payment_methods import canonical_payment_method_id, ledger_group
from ..errors import AppError
from ..fingerprints import make_transaction_fingerprint
from ..msat import btc_to_msat, msat_to_btc
from . import btcpay_ledger as core_btcpay_ledger

# Tables whose rows bind a transaction's wallet or quantity. A row they
# reference keeps its wallet and fee: moving it would silently change
# reviewed custody, pairs, loans or source-of-funds evidence.
# Soft-deleted pairs, swap payouts and loan legs bind nothing.
_BINDING_REFERENCES = (
    ("custody_component_legs", ("transaction_id", "anchor_transaction_id"), ""),
    ("custody_component_transaction_memberships", ("transaction_id",), ""),
    ("custody_gap_review_transactions", ("transaction_id",), ""),
    ("transaction_pairs", ("out_transaction_id", "in_transaction_id"), " AND deleted_at IS NULL"),
    ("direct_swap_payouts", ("out_transaction_id",), " AND deleted_at IS NULL"),
    ("loan_legs", ("transaction_id",), " AND deleted_at IS NULL"),
    ("source_funds_links", ("from_transaction_id", "to_transaction_id"), ""),
    ("source_funds_cases", ("target_transaction_id",), ""),
)


def _raw(value: Any) -> dict[str, Any]:
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _bark_method(payment_method_id: Any) -> bool:
    return canonical_payment_method_id(payment_method_id) == BARK_PAYMENT_METHOD_ID


def _lightning_method(payment_method_id: Any) -> bool:
    return ledger_group(payment_method_id) == "lightning"


def _provenance_index(conn: sqlite3.Connection, profile_id: str, store_id: str) -> dict[tuple, list[tuple[str, int | None]]]:
    """Provenance keys (and amounts) of the store's Bark and Lightning payments and payouts."""

    index: dict[tuple, list[tuple[str, int | None]]] = {}

    def add(key: tuple, row: sqlite3.Row) -> None:
        keys = index.setdefault(key, [])
        if all(existing != row["stable_key"] for existing, _ in keys):
            keys.append((row["stable_key"], row["amount"]))

    rows = conn.execute(
        """
        SELECT record_type, stable_key, invoice_id, payment_id, payment_method_id, txid, payment_hash, amount
        FROM btcpay_provenance_records
        WHERE profile_id = ? AND store_id = ? AND record_type IN ('payment', 'payout')
        ORDER BY stable_key ASC
        """,
        (profile_id, store_id),
    ).fetchall()
    for row in rows:
        bark = _bark_method(row["payment_method_id"])
        lightning = _lightning_method(row["payment_method_id"])
        if not (bark or lightning):
            continue
        payment_hash = str(row["payment_hash"] or "").lower() or None
        if row["record_type"] == "payout":
            if payment_hash:
                add(("outbound", "payment_hash", payment_hash), row)
            if bark and row["txid"]:
                add(("outbound", "transaction_hash", str(row["txid"]).lower()), row)
            continue
        keys = payment_keys(row["payment_id"]) if bark else {}
        if lightning and payment_hash:
            keys = {"payment_hash": payment_hash}
        if "outpoint" in keys:
            add(("inbound", "outpoint", keys["outpoint"]), row)
        if "payment_hash" in keys:
            add(("inbound", "payment_hash", keys["payment_hash"]), row)
        if "movement_id" in keys and keys["invoice_id"] == row["invoice_id"]:
            add(("inbound", "movement", keys["invoice_id"], keys["movement_id"]), row)
    return index


def _candidates(record: Mapping[str, Any], index: Mapping[tuple, list[tuple[str, int | None]]]) -> list[str]:
    keys = record.get("_bark_keys") or {}
    direction = record["direction"]
    raw = _raw(record.get("raw_json"))
    lookups: list[tuple] = []
    if direction == "inbound":
        if keys.get("outpoint"):
            lookups.append(("inbound", "outpoint", keys["outpoint"]))
        if keys.get("payment_hash"):
            lookups.append(("inbound", "payment_hash", keys["payment_hash"]))
        if keys.get("invoice_id") and keys.get("movement_id") is not None:
            lookups.append(("inbound", "movement", keys["invoice_id"], keys["movement_id"]))
    else:
        if keys.get("payment_hash"):
            lookups.append(("outbound", "payment_hash", keys["payment_hash"]))
        if raw.get("transaction_hash"):
            lookups.append(("outbound", "transaction_hash", str(raw["transaction_hash"]).lower()))
    for lookup in lookups:
        found = index.get(lookup)
        if found:
            # One movement can pay several payments (one per VTXO): the
            # amount tells them apart, or none is taken.
            if len(found) > 1:
                amount = btc_to_msat(record["amount"])
                same = [key for key, value in found if value is not None and abs(int(value)) == amount]
                if len(same) == 1:
                    return same
            return [key for key, _ in found]
    return []


def stored_identities(conn: sqlite3.Connection, profile_id: str, wallet_id: str) -> dict[tuple[str, str], dict[str, Any]]:
    """``(bark_row_id, part)`` -> the stored row booked from it."""

    stored: dict[tuple[str, str], dict[str, Any]] = {}
    for row in conn.execute(
        """
        SELECT id, external_id, occurred_at, direction, asset, amount, fee, raw_json
        FROM transactions WHERE profile_id = ? AND wallet_id = ? AND external_id IS NOT NULL
        ORDER BY created_at ASC, id ASC
        """,
        (profile_id, wallet_id),
    ).fetchall():
        raw = _raw(row["raw_json"])
        if raw.get("source") == BARK_RAW_SOURCE and raw.get("bark_row_id"):
            stored.setdefault((str(raw["bark_row_id"]), str(raw.get("part") or PART_PRINCIPAL)), dict(row))
    return stored


def assign_identities(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_id: str,
    *,
    store_id: str,
    records: Sequence[dict[str, Any]],
) -> dict[str, int]:
    """Set each record's ``id``; move stored fallback ids to a provenance key.

    A provenance key two rows would share is ambiguous, so neither takes it.
    """

    index = _provenance_index(conn, profile_id, store_id)
    claims: dict[str, int] = {}
    wanted: list[str | None] = []
    for record in records:
        found = _candidates(record, index) if record["_bark_part"] == PART_PRINCIPAL else []
        key = found[0] if len(found) == 1 else None
        wanted.append(key)
        if key:
            claims[key] = claims.get(key, 0) + 1
    stored = stored_identities(conn, profile_id, wallet_id)
    counts = {"linked": 0, "ambiguous": 0, "rekeyed": 0}
    for record, key in zip(records, wanted):
        if key and claims[key] > 1:
            counts["ambiguous"] += 1
            key = None
        desired = key or fallback_identity(store_id, record["_bark_row_id"], record["_bark_part"])
        counts["linked"] += int(key is not None)
        existing = stored.get((record["_bark_row_id"], record["_bark_part"]))
        if existing is None:
            record["id"] = record["txid"] = desired
            continue
        current = existing["external_id"]
        if current != desired and key and is_fallback_identity(store_id, current) and not _identity_taken(conn, wallet_id, desired):
            conn.execute(
                "UPDATE transactions SET external_id = ?, fingerprint = ? WHERE id = ?",
                (
                    desired,
                    make_transaction_fingerprint(
                        wallet_id,
                        desired,
                        existing["occurred_at"],
                        existing["direction"],
                        existing["asset"],
                        msat_to_btc(existing["amount"]),
                        msat_to_btc(existing["fee"]),
                    ),
                    existing["id"],
                ),
            )
            counts["rekeyed"] += 1
            current = desired
        record["id"] = record["txid"] = current
    return counts


def _identity_taken(conn: sqlite3.Connection, wallet_id: str, external_id: str) -> bool:
    return conn.execute(
        "SELECT 1 FROM transactions WHERE wallet_id = ? AND external_id = ?",
        (wallet_id, external_id),
    ).fetchone() is not None


def bark_networks(conn: sqlite3.Connection, profile_id: str, store_id: str) -> list[str]:
    """``main``/``test`` from the Ark addresses the store's Bark payments went to."""

    found = set()
    for row in conn.execute(
        """
        SELECT payment_method_id, destination FROM btcpay_provenance_records
        WHERE profile_id = ? AND store_id = ? AND record_type = 'payment' AND destination IS NOT NULL
        """,
        (profile_id, store_id),
    ).fetchall():
        if not _bark_method(row["payment_method_id"]):
            continue
        destination = str(row["destination"]).strip().lower()
        if destination.startswith("tark1"):
            found.add("test")
        elif destination.startswith("ark1"):
            found.add("main")
    return sorted(found)


def require_book_network(conn: sqlite3.Connection, profile_id: str, store_id: str) -> None:
    """Refuse a Bark wallet whose payments went to another network than the book's."""

    from .book_network import resolve_book_environment

    try:
        binding = resolve_book_environment(conn, profile_id)
    except sqlite3.OperationalError:
        return
    if binding.get("state") != "bound":
        return
    scope = "main" if binding.get("environment") == "main" else "test"
    networks = bark_networks(conn, profile_id, store_id)
    if networks and networks != [scope]:
        raise AppError(
            f"The Bark wallet of store {store_id} is on a different network than this book",
            code="book_network_mismatch",
            hint=(
                "Its payments went to Ark addresses of another network (`ark1` is mainnet, `tark1` a test "
                "network). Connect this store in a book bound to that network."
            ),
            details={"store_id": store_id, "bark_networks": networks, "book_environment": binding.get("environment")},
            retryable=False,
        )


def binding_references(conn: sqlite3.Connection, transaction_id: str) -> list[str]:
    """Tables that bind this transaction's wallet or quantity."""

    found = []
    for table, columns, live in _BINDING_REFERENCES:
        clause = " OR ".join(f"{column} = ?" for column in columns)
        if conn.execute(
            f"SELECT 1 FROM {table} WHERE ({clause}){live} LIMIT 1", (transaction_id,) * len(columns)
        ).fetchone():
            found.append(table)
    return found


def _conflict(message: str, *, reason: str, hint: str, details: Mapping[str, Any]) -> AppError:
    return AppError(
        message,
        code="bark_migration_conflict",
        hint=hint,
        details={"reason": reason, **details},
        retryable=False,
    )


def _record_msat(record: Mapping[str, Any], field: str) -> int:
    return int(btc_to_msat(record[field]) or 0)


def _stamp(conn: sqlite3.Connection, row: Mapping[str, Any], record: Mapping[str, Any], *, wallet_id: str) -> None:
    """Keep ``row`` as ``record``: its id, notes and reviews stay; Bark's fee and evidence apply."""

    fee = _record_msat(record, "fee")
    conn.execute(
        """
        UPDATE transactions
        SET wallet_id = ?, fee = ?, amount_includes_fee = 0, raw_json = ?, payment_hash = ?,
            payment_hash_source = ?, fingerprint = ?
        WHERE id = ?
        """,
        (
            wallet_id,
            fee,
            record["raw_json"],
            record.get("payment_hash") or row["payment_hash"],
            record.get("payment_hash_source") or row["payment_hash_source"],
            make_transaction_fingerprint(
                wallet_id,
                row["external_id"],
                row["occurred_at"],
                row["direction"],
                row["asset"],
                msat_to_btc(row["amount"]),
                msat_to_btc(fee),
            ),
            row["id"],
        ),
    )


def _match(row: Mapping[str, Any], record: Mapping[str, Any] | None) -> str | None:
    """Why ``record`` cannot keep ``row``, or None."""

    if record is None:
        return "unmatched"
    if record["direction"] != row["direction"] or record["asset"] != row["asset"]:
        return "direction_mismatch"
    if _record_msat(record, "amount") != int(row["amount"] or 0):
        return "amount_mismatch"
    return None


def migrate_payment_ledger(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    """Keep a ``BTC-ARK`` ledger's rows as the Bark records they booked.

    ``records`` hold the whole confirmed wallet history with identities set.
    Raises before any write when an active ledger row cannot be kept.
    """

    by_id: dict[str, list[Mapping[str, Any]]] = {}
    for record in records:
        by_id.setdefault(record["id"], []).append(record)
    rows = conn.execute(
        """
        SELECT * FROM transactions
        WHERE profile_id = ? AND wallet_id = ? AND excluded = 0
        ORDER BY occurred_at ASC, id ASC
        """,
        (profile["id"], wallet["id"]),
    ).fetchall()
    plan: list[tuple[Mapping[str, Any], Mapping[str, Any]]] = []
    problems: list[dict[str, Any]] = []
    for row in rows:
        if _raw(row["raw_json"]).get("source") != core_btcpay_ledger.LEDGER_RAW_SOURCE:
            continue
        matches = by_id.get(row["external_id"]) or []
        record = matches[0] if len(matches) == 1 else None
        reason = "ambiguous" if len(matches) > 1 else _match(row, record)
        extra: dict[str, Any] = {}
        if reason is None and _record_msat(record, "fee") != int(row["fee"] or 0):
            # Adding Bark's fee changes the row's quantity; reviewed evidence on it would not follow.
            bound = binding_references(conn, row["id"])
            if bound:
                reason, extra = "bound_by_review", {"tables": bound}
        if reason:
            problems.append({"transaction_id": row["id"], "external_id": row["external_id"], "reason": reason, **extra})
            continue
        plan.append((row, record))
    if problems:
        raise _conflict(
            f"{len(problems)} payment(s) booked by '{wallet['label']}' have no single matching row in the Bark wallet",
            reason="unmatched_ledger_rows",
            hint=(
                "Kassiber keeps every booked payment when it switches this store to Bark's wallet history, and "
                "refuses rather than guess. Check the listed payments in BTCPay; excluding a payment that was "
                "booked by mistake lets the switch go ahead. Until then the payment ledger stays as it is."
            ),
            details={"wallet": wallet["label"], "problems": problems},
        )
    for row, record in plan:
        _stamp(conn, row, record, wallet_id=str(wallet["id"]))
    return {"kept": len(plan)}


def adopt_lightning_ledger_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet: Mapping[str, Any],
    *,
    backend_name: str,
    store_id: str,
    records: Sequence[Mapping[str, Any]],
) -> int:
    """Move ``BTC-LN`` ledger rows of payments the Bark wallet books into it.

    Raises when another wallet books one of them, or the ledger row is bound
    by custody, pairing, loan or source-of-funds evidence.
    """

    moved = 0
    for record in records:
        payment_hash = str(record.get("payment_hash") or "").lower()
        if not payment_hash:
            continue
        rows = conn.execute(
            """
            SELECT t.*, w.label AS wallet_label, w.config_json AS wallet_config
            FROM transactions t JOIN wallets w ON w.id = t.wallet_id
            WHERE t.profile_id = ? AND t.wallet_id != ? AND t.excluded = 0
              AND LOWER(t.payment_hash) = ? AND t.direction = ?
            ORDER BY t.id ASC
            """,
            (profile_id, wallet["id"], payment_hash, record["direction"]),
        ).fetchall()
        for row in rows:
            config = _raw(row["wallet_config"])
            raw = _raw(row["raw_json"])
            ledger_row = (
                raw.get("source") == core_btcpay_ledger.LEDGER_RAW_SOURCE
                and str(raw.get("store_id") or "") == store_id
                and str(config.get("backend") or "").strip().lower() == backend_name.strip().lower()
                and core_btcpay_ledger.is_payment_ledger_config(config)
            )
            details = {
                "payment_hash": payment_hash,
                "transaction_id": row["id"],
                "wallet": row["wallet_label"],
                "bark_row_id": record["_bark_row_id"],
            }
            if not ledger_row:
                raise _conflict(
                    f"'{row['wallet_label']}' already books a Lightning payment the Bark wallet settles",
                    reason="booked_by_other_wallet",
                    hint=(
                        "A payment can settle in one wallet only. Check which wallet really received it and "
                        "exclude the other copy, then sync again."
                    ),
                    details=details,
                )
            reason = None
            if row["external_id"] != record["id"]:
                reason = "identity_mismatch"
            elif _identity_taken(conn, str(wallet["id"]), record["id"]):
                reason = "already_booked_by_bark"
            elif _match(row, record):
                reason = _match(row, record)
            bound = binding_references(conn, row["id"])
            if reason is None and bound:
                reason, details = "bound_by_review", {**details, "tables": bound}
            if reason:
                raise _conflict(
                    f"A Lightning payment in '{row['wallet_label']}' cannot move to the Bark wallet that settled it",
                    reason=reason,
                    hint=(
                        "The Bark wallet owns Lightning payments that settle into it, so the Lightning ledger's copy "
                        "moves there with its notes and reviews. Resolve the listed review first, or exclude one copy."
                    ),
                    details=details,
                )
            _stamp(conn, row, record, wallet_id=str(wallet["id"]))
            moved += 1
    return moved


def bark_config(config: Mapping[str, Any]) -> dict[str, Any]:
    """A ``BTC-ARK`` payment ledger's config as a Bark wallet import."""

    switched = {
        key: value
        for key, value in config.items()
        if key not in {core_btcpay_ledger.SOURCE_MODE_CONFIG_KEY, core_btcpay_ledger.LEDGER_PAYMENT_METHODS_CONFIG_KEY}
    }
    switched["payment_method_id"] = BARK_PAYMENT_METHOD_ID
    return switched


def is_bark_ledger(config: Mapping[str, Any]) -> bool:
    """A payment ledger that books only the Bark rail."""

    methods = core_btcpay_ledger.ledger_payment_method_ids(config) if core_btcpay_ledger.is_payment_ledger_config(config) else []
    return bool(methods) and all(_bark_method(method) for method in methods)


__all__ = [
    "adopt_lightning_ledger_rows",
    "bark_networks",
    "require_book_network",
    "assign_identities",
    "bark_config",
    "binding_references",
    "is_bark_ledger",
    "migrate_payment_ledger",
    "stored_identities",
]

