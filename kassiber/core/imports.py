from __future__ import annotations

"""Import orchestration helpers above the parser-only `kassiber.importers` boundary."""

import json
import os
import sqlite3
import uuid
from dataclasses import dataclass
from typing import Any, Callable, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..fingerprints import make_transaction_fingerprint
from . import pricing
from ..importers import is_btcpay_format, is_phoenix_format, load_import_records
from ..msat import btc_to_msat, dec
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, parse_timestamp
from ..util import str_or_none
from ..wallet_descriptors import normalize_asset_code

INBOUND_DIRECTIONS = {"in", "inbound", "receive", "received", "deposit", "credit", "buy"}
OUTBOUND_DIRECTIONS = {"out", "outbound", "send", "sent", "withdrawal", "withdraw", "debit", "sell"}
FIAT_PRICE_SOURCE_IMPORT = pricing.LEGACY_SOURCE_IMPORT
FIAT_PRICE_SOURCE_RATES_CACHE = pricing.LEGACY_SOURCE_RATES_CACHE

ImportRow = Mapping[str, Any]
TagRow = Mapping[str, Any]
EnsureTagRow = Callable[[sqlite3.Connection, str, str, str, str], tuple[TagRow, bool]]
InvalidateJournals = Callable[[sqlite3.Connection, str], None]


@dataclass(frozen=True)
class ImportCoordinatorHooks:
    ensure_tag_row: EnsureTagRow
    invalidate_journals: InvalidateJournals


def normalize_import_direction(direction: Any, amount: Any) -> str:
    if direction:
        value = str(direction).strip().lower()
        if value in INBOUND_DIRECTIONS:
            return "inbound"
        if value in OUTBOUND_DIRECTIONS:
            return "outbound"
        raise AppError(f"Unsupported direction '{direction}'")
    return "outbound" if dec(amount) < 0 else "inbound"


def _find_existing_transaction(
    conn: sqlite3.Connection,
    wallet_id: str,
    normalized: Mapping[str, Any],
    fingerprint: str,
):
    existing = conn.execute(
        """
        SELECT id, fingerprint, occurred_at, confirmed_at, fiat_rate, fiat_value,
               fiat_price_source, fiat_rate_exact, fiat_value_exact,
               pricing_source_kind, pricing_provider, pricing_pair, pricing_timestamp,
               pricing_fetched_at, pricing_granularity, pricing_method,
               pricing_external_ref, pricing_quality,
               kind, description, counterparty, raw_json
        FROM transactions
        WHERE fingerprint = ?
        """,
        (fingerprint,),
    ).fetchone()
    if existing or not normalized["external_id"]:
        return existing
    return conn.execute(
        """
        SELECT id, fingerprint, occurred_at, confirmed_at, fiat_rate, fiat_value,
               fiat_price_source, fiat_rate_exact, fiat_value_exact,
               pricing_source_kind, pricing_provider, pricing_pair, pricing_timestamp,
               pricing_fetched_at, pricing_granularity, pricing_method,
               pricing_external_ref, pricing_quality,
               kind, description, counterparty, raw_json
        FROM transactions
        WHERE wallet_id = ?
          AND external_id = ?
          AND direction = ?
          AND asset = ?
          AND amount = ?
          AND fee = ?
        ORDER BY created_at DESC
        LIMIT 1
        """,
        (
            wallet_id,
            normalized["external_id"],
            normalized["direction"],
            normalized["asset"],
            btc_to_msat(normalized["amount"]),
            btc_to_msat(normalized["fee"]),
        ),
    ).fetchone()


PRICE_COLUMNS = (
    "fiat_rate",
    "fiat_value",
    "fiat_rate_exact",
    "fiat_value_exact",
    "fiat_price_source",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
)


def _transaction_merge_updates(existing: Mapping[str, Any], normalized: Mapping[str, Any], fingerprint: str):
    updates = {}
    if (
        existing["occurred_at"] == UNKNOWN_OCCURRED_AT
        and normalized["occurred_at"] != UNKNOWN_OCCURRED_AT
    ):
        updates["occurred_at"] = normalized["occurred_at"]
    stored_occurred_at = updates.get("occurred_at", existing["occurred_at"])
    if existing["fingerprint"] != fingerprint and stored_occurred_at == normalized["occurred_at"]:
        updates["fingerprint"] = fingerprint

    confirmed_at_added = (
        existing["confirmed_at"] in (None, "")
        and normalized["confirmed_at"] is not None
    )
    if confirmed_at_added:
        updates["confirmed_at"] = normalized["confirmed_at"]

    has_existing_price = (
        existing["fiat_rate"] is not None
        or existing["fiat_value"] is not None
        or existing["fiat_rate_exact"] is not None
        or existing["fiat_value_exact"] is not None
    )
    has_import_price = normalized["pricing_source_kind"] is not None
    incoming_priority = pricing.priority_for(normalized["pricing_source_kind"])
    existing_priority = pricing.priority_for(
        existing["pricing_source_kind"],
        existing["fiat_price_source"],
    )
    if has_import_price and (not has_existing_price or incoming_priority >= existing_priority):
        updates.update({column: normalized[column] for column in PRICE_COLUMNS})
    elif confirmed_at_added and existing["fiat_price_source"] == FIAT_PRICE_SOURCE_RATES_CACHE:
        updates.update({column: None for column in PRICE_COLUMNS})

    if not existing["kind"] and normalized["kind"]:
        updates["kind"] = normalized["kind"]
    if not existing["description"] and normalized["description"]:
        updates["description"] = normalized["description"]
    if not existing["counterparty"] and normalized["counterparty"]:
        updates["counterparty"] = normalized["counterparty"]
    if updates and normalized["raw_json"] and normalized["raw_json"] != existing["raw_json"]:
        updates["raw_json"] = normalized["raw_json"]
    return updates


def normalize_import_record(record: ImportRow, source_label: str = "") -> dict[str, Any]:
    raw_amount = dec(record.get("amount"))
    direction = normalize_import_direction(record.get("direction"), raw_amount)
    amount = abs(raw_amount)
    fee = abs(dec(record.get("fee"), "0"))
    fiat_rate = record.get("fiat_rate")
    fiat_value = record.get("fiat_value")
    has_import_price = fiat_rate not in (None, "") or fiat_value not in (None, "")
    rate = dec(fiat_rate) if fiat_rate not in (None, "") else None
    value = dec(fiat_value) if fiat_value not in (None, "") else None
    if value is None and rate is not None:
        value = amount * rate
    source_kind = None
    quality = None
    if has_import_price:
        source_kind = pricing.infer_import_source_kind(source_label, record)
        quality = str(record.get("pricing_quality") or "").strip().lower() or pricing.import_quality(source_kind)
    raw_json = record.get("raw_json")
    if raw_json is None:
        raw_json = json.dumps(json_ready(record), sort_keys=True)
    elif not isinstance(raw_json, str):
        raw_json = json.dumps(json_ready(raw_json), sort_keys=True)
    occurred_at = parse_timestamp(record.get("occurred_at") or record.get("timestamp") or record.get("date"))
    confirmed_at = record.get("confirmed_at")
    confirmed_at = parse_timestamp(confirmed_at) if confirmed_at not in (None, "") else None
    if record.get("pricing_timestamp") not in (None, ""):
        pricing_timestamp = parse_timestamp(record.get("pricing_timestamp"))
    elif has_import_price:
        pricing_timestamp = confirmed_at or occurred_at
    else:
        pricing_timestamp = None
    payload = pricing.pricing_payload(
        rate=rate,
        value=value,
        source_kind=source_kind,
        quality=quality,
        provider=str_or_none(record.get("pricing_provider") or record.get("provider")),
        pair=str_or_none(record.get("pricing_pair") or record.get("pair")),
        pricing_timestamp=pricing_timestamp,
        fetched_at=parse_timestamp(record.get("pricing_fetched_at"))
        if record.get("pricing_fetched_at") not in (None, "")
        else None,
        granularity=str_or_none(record.get("pricing_granularity") or record.get("granularity")),
        method=str_or_none(record.get("pricing_method") or record.get("method")),
        external_ref=str_or_none(record.get("pricing_external_ref") or record.get("external_ref")),
    )
    return {
        "external_id": str(record.get("txid") or record.get("id") or ""),
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": normalize_asset_code(record.get("asset") or "BTC"),
        "amount": amount,
        "fee": fee,
        **payload,
        "kind": record.get("kind"),
        "description": record.get("description"),
        "counterparty": record.get("counterparty"),
        "raw_json": raw_json,
    }


def insert_wallet_records(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    source_label: str,
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, Any]:
    imported = 0
    skipped = 0
    for record in records:
        normalized = normalize_import_record(record, source_label=source_label)
        fingerprint = make_transaction_fingerprint(
            wallet["id"],
            normalized["external_id"],
            normalized["occurred_at"],
            normalized["direction"],
            normalized["asset"],
            normalized["amount"],
            normalized["fee"],
        )
        existing = _find_existing_transaction(conn, wallet["id"], normalized, fingerprint)
        if existing:
            updates = _transaction_merge_updates(existing, normalized, fingerprint)
            if updates:
                assignments = ", ".join(f"{column} = ?" for column in updates)
                conn.execute(
                    f"UPDATE transactions SET {assignments} WHERE id = ?",
                    (*updates.values(), existing["id"]),
                )
            skipped += 1
            continue
        tx_id = str(uuid.uuid4())
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, fiat_price_source, fiat_rate_exact,
                fiat_value_exact, pricing_source_kind, pricing_provider, pricing_pair,
                pricing_timestamp, pricing_fetched_at, pricing_granularity,
                pricing_method, pricing_external_ref, pricing_quality, kind, description,
                counterparty, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                normalized["external_id"] or None,
                fingerprint,
                normalized["occurred_at"],
                normalized["confirmed_at"],
                normalized["direction"],
                normalized["asset"],
                btc_to_msat(normalized["amount"]),
                btc_to_msat(normalized["fee"]),
                profile["fiat_currency"],
                normalized["fiat_rate"],
                normalized["fiat_value"],
                normalized["fiat_price_source"],
                normalized["fiat_rate_exact"],
                normalized["fiat_value_exact"],
                normalized["pricing_source_kind"],
                normalized["pricing_provider"],
                normalized["pricing_pair"],
                normalized["pricing_timestamp"],
                normalized["pricing_fetched_at"],
                normalized["pricing_granularity"],
                normalized["pricing_method"],
                normalized["pricing_external_ref"],
                normalized["pricing_quality"],
                normalized["kind"],
                normalized["description"],
                normalized["counterparty"],
                normalized["raw_json"],
                now_iso(),
            ),
        )
        imported += 1
    hooks.invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return {
        "wallet": wallet["label"],
        "source": source_label,
        "imported": imported,
        "skipped": skipped,
    }


def import_records_into_wallet(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    source_label: str,
    hooks: ImportCoordinatorHooks,
    *,
    apply_btcpay: bool = False,
    apply_phoenix: bool = False,
    commit: bool = True,
) -> dict[str, Any]:
    outcome = insert_wallet_records(
        conn,
        profile,
        wallet,
        records,
        source_label,
        hooks,
        commit=False,
    )
    if apply_btcpay:
        outcome.update(apply_btcpay_metadata(conn, profile, wallet, records, hooks, commit=False))
    if apply_phoenix:
        outcome.update(apply_phoenix_metadata(conn, profile, wallet, records, hooks, commit=False))
    if commit:
        conn.commit()
    return outcome


def apply_phoenix_metadata(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, int]:
    notes_set = 0
    tags_added = 0
    tags_created = 0
    for record in records:
        txid = record.get("txid")
        if not txid:
            continue
        tx = conn.execute(
            """
            SELECT id, note
            FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (profile["id"], wallet["id"], txid),
        ).fetchone()
        if not tx:
            continue
        description = str_or_none(record.get("_phoenix_description"))
        if description and not tx["note"]:
            conn.execute(
                "UPDATE transactions SET note = ? WHERE id = ?",
                (description, tx["id"]),
            )
            notes_set += 1
        phoenix_type = str_or_none(record.get("_phoenix_type"))
        if phoenix_type:
            tag, created = hooks.ensure_tag_row(
                conn,
                profile["workspace_id"],
                profile["id"],
                phoenix_type,
                phoenix_type,
            )
            if created:
                tags_created += 1
            before = conn.total_changes
            conn.execute(
                "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                (tx["id"], tag["id"]),
            )
            if conn.total_changes > before:
                tags_added += 1
    if commit:
        conn.commit()
    return {
        "phoenix_notes_set": notes_set,
        "phoenix_tags_added": tags_added,
        "phoenix_tags_created": tags_created,
    }


def apply_btcpay_metadata(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, int]:
    notes_set = 0
    tags_added = 0
    tags_created = 0
    for record in records:
        txid = record.get("txid")
        if not txid:
            continue
        tx = conn.execute(
            """
            SELECT id, note
            FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (profile["id"], wallet["id"], txid),
        ).fetchone()
        if not tx:
            continue
        comment = str_or_none(record.get("_btcpay_comment"))
        if comment and not tx["note"]:
            conn.execute("UPDATE transactions SET note = ? WHERE id = ?", (comment, tx["id"]))
            notes_set += 1
        for label in record.get("_btcpay_labels", []):
            tag, created = hooks.ensure_tag_row(
                conn,
                profile["workspace_id"],
                profile["id"],
                label,
                label,
            )
            if created:
                tags_created += 1
            before = conn.total_changes
            conn.execute(
                "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                (tx["id"], tag["id"]),
            )
            if conn.total_changes > before:
                tags_added += 1
    if commit:
        conn.commit()
    return {
        "btcpay_notes_set": notes_set,
        "btcpay_tags_added": tags_added,
        "btcpay_tags_created": tags_created,
    }


def import_file_into_wallet(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    file_path: str,
    input_format: str,
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, Any]:
    records = load_import_records(file_path, input_format)
    outcome = import_records_into_wallet(
        conn,
        profile,
        wallet,
        records,
        f"file:{input_format}",
        hooks,
        apply_btcpay=is_btcpay_format(input_format),
        apply_phoenix=is_phoenix_format(input_format),
        commit=commit,
    )
    outcome["input_format"] = input_format
    outcome["file"] = os.path.abspath(file_path)
    return outcome


__all__ = [
    "ImportCoordinatorHooks",
    "apply_btcpay_metadata",
    "apply_phoenix_metadata",
    "import_file_into_wallet",
    "import_records_into_wallet",
    "insert_wallet_records",
    "make_transaction_fingerprint",
    "normalize_import_direction",
    "normalize_import_record",
]
