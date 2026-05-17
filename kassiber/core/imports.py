from __future__ import annotations

"""Import orchestration helpers above the parser-only `kassiber.importers` boundary."""

import contextvars
import json
import os
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, Callable, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..fingerprints import make_transaction_fingerprint
from . import pricing
from ..importers import (
    is_bullbitcoin_format,
    is_btcpay_format,
    is_phoenix_format,
    is_pocketbitcoin_format,
    is_river_format,
    load_import_records,
)
from ..msat import btc_to_msat, dec
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, parse_timestamp
from ..util import str_or_none
from ..wallet_descriptors import normalize_asset_code

INBOUND_DIRECTIONS = {"in", "inbound", "receive", "received", "deposit", "credit", "buy"}
OUTBOUND_DIRECTIONS = {"out", "outbound", "send", "sent", "withdrawal", "withdraw", "debit", "sell"}
FIAT_PRICE_SOURCE_RATES_CACHE = pricing.LEGACY_SOURCE_RATES_CACHE

ImportRow = Mapping[str, Any]
TagRow = Mapping[str, Any]
EnsureTagRow = Callable[[sqlite3.Connection, str, str, str, str], tuple[TagRow, bool]]
InvalidateJournals = Callable[[sqlite3.Connection, str], None]
BULLBITCOIN_IMPORT_MODE_RELEVANT = "relevant"
BULLBITCOIN_IMPORT_MODE_FULL = "full"
BULLBITCOIN_IMPORT_MODES = {
    BULLBITCOIN_IMPORT_MODE_RELEVANT,
    BULLBITCOIN_IMPORT_MODE_FULL,
}
BULLBITCOIN_RECONCILIATION_TAGS = {
    "matched": ("bullbitcoin-matched", "Bull Bitcoin matched"),
    "unmatched": ("bullbitcoin-wallet-gap", "Bull Bitcoin wallet gap"),
    "ambiguous": ("bullbitcoin-ambiguous", "Bull Bitcoin ambiguous"),
}
POCKETBITCOIN_RECONCILIATION_TAGS = {
    "matched": ("pocketbitcoin-matched", "Pocket Bitcoin matched"),
    "unmatched": ("pocketbitcoin-wallet-gap", "Pocket Bitcoin wallet gap"),
    "ambiguous": ("pocketbitcoin-ambiguous", "Pocket Bitcoin ambiguous"),
}


ProgressCallback = Callable[[Mapping[str, Any]], None]

# Contextvar threaded by the daemon when it wants long-running imports to
# emit row-count progress over the JSONL stream. The CLI leaves this empty
# so no behavior change for `kassiber wallets sync` from a terminal.
sync_progress_emitter: contextvars.ContextVar[ProgressCallback | None] = (
    contextvars.ContextVar("kassiber.sync_progress_emitter", default=None)
)


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


_EXISTING_TRANSACTION_COLUMNS = """
       id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
       confirmed_at, fiat_rate, fiat_value, fiat_price_source, fiat_rate_exact,
       fiat_value_exact, pricing_source_kind, pricing_provider, pricing_pair,
       pricing_timestamp, pricing_fetched_at, pricing_granularity, pricing_method,
       pricing_external_ref, pricing_quality, kind, description, counterparty,
       raw_json, payment_hash, payment_hash_source
"""


def _find_existing_transaction(
    conn: sqlite3.Connection,
    wallet_id: str,
    normalized: Mapping[str, Any],
    fingerprint: str,
):
    existing = conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}
        FROM transactions
        WHERE fingerprint = ?
        """,
        (fingerprint,),
    ).fetchone()
    if existing or not normalized["external_id"]:
        return existing
    existing = conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}
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
    if existing or normalized["pricing_source_kind"] != pricing.SOURCE_EXCHANGE_EXECUTION:
        return existing
    return conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}
        FROM transactions
        WHERE wallet_id = ?
          AND external_id = ?
          AND direction = ?
          AND asset = ?
          AND amount = ?
        ORDER BY created_at DESC
        LIMIT 1
        """,
        (
            wallet_id,
            normalized["external_id"],
            normalized["direction"],
            normalized["asset"],
            btc_to_msat(normalized["amount"]),
        ),
    ).fetchone()


def _single_match(rows: Sequence[sqlite3.Row]) -> sqlite3.Row | None:
    if len(rows) == 1:
        return rows[0]
    return None


def _timestamp_window(value: str, tolerance_seconds: int) -> tuple[str, str] | None:
    if not value or tolerance_seconds <= 0:
        return None
    raw = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        center = datetime.fromisoformat(raw)
    except ValueError:
        return None
    if center.tzinfo is None:
        center = center.replace(tzinfo=timezone.utc)
    else:
        center = center.astimezone(timezone.utc)
    delta = timedelta(seconds=tolerance_seconds)
    start = (center - delta).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    end = (center + delta).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    return start, end


def _find_existing_profile_transaction_by_economics(
    conn: sqlite3.Connection,
    profile_id: str,
    normalized: Mapping[str, Any],
    *,
    exclude_wallet_id: str | None = None,
) -> tuple[sqlite3.Row | None, str]:
    """Find exchange evidence matches when the provider export has no txid."""
    if not normalized.get("match_without_external_id"):
        return None, "unmatched"
    wallet_filter = "AND wallet_id != ?" if exclude_wallet_id else ""
    params: list[Any] = [
        profile_id,
        normalized["direction"],
        normalized["asset"],
        btc_to_msat(normalized["amount"]),
    ]
    if exclude_wallet_id:
        params.append(exclude_wallet_id)
    tolerance_seconds = int(normalized.get("match_time_tolerance_seconds") or 0)
    window = _timestamp_window(normalized["occurred_at"], tolerance_seconds)
    time_filter = ""
    if window is not None:
        time_filter = "AND occurred_at BETWEEN ? AND ?"
        params.extend(window)
    rows = conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}, wallet_label
        FROM (
            SELECT t.*, w.label AS wallet_label
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
        )
        WHERE profile_id = ?
          AND direction = ?
          AND asset = ?
          AND amount = ?
          {wallet_filter}
          {time_filter}
        ORDER BY created_at DESC
        LIMIT 2
        """,
        tuple(params),
    ).fetchall()
    match = _single_match(rows)
    if match:
        return match, "matched"
    if len(rows) > 1:
        return None, "ambiguous"
    return None, "unmatched"


def _find_existing_profile_transaction_result(
    conn: sqlite3.Connection,
    profile_id: str,
    normalized: Mapping[str, Any],
    *,
    exclude_wallet_id: str | None = None,
) -> tuple[sqlite3.Row | None, str]:
    """Find one unambiguous book-wide transaction for exchange evidence."""
    if not normalized["external_id"]:
        return _find_existing_profile_transaction_by_economics(
            conn,
            profile_id,
            normalized,
            exclude_wallet_id=exclude_wallet_id,
        )
    amount_msat = btc_to_msat(normalized["amount"])
    fee_msat = btc_to_msat(normalized["fee"])
    wallet_filter = "AND wallet_id != ?" if exclude_wallet_id else ""
    base_params: list[Any] = [
        profile_id,
        normalized["external_id"],
        normalized["direction"],
        normalized["asset"],
        amount_msat,
    ]
    if exclude_wallet_id:
        base_params.append(exclude_wallet_id)
    exact_fee_rows = conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}, wallet_label
        FROM (
            SELECT t.*, w.label AS wallet_label
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
        )
        WHERE profile_id = ?
          AND external_id = ?
          AND direction = ?
          AND asset = ?
          AND amount = ?
          AND fee = ?
          {wallet_filter}
        ORDER BY created_at DESC
        LIMIT 2
        """,
        (
            profile_id,
            normalized["external_id"],
            normalized["direction"],
            normalized["asset"],
            amount_msat,
            fee_msat,
            *((exclude_wallet_id,) if exclude_wallet_id else ()),
        ),
    ).fetchall()
    match = _single_match(exact_fee_rows)
    if match or len(exact_fee_rows) > 1:
        return match, "matched" if match else "ambiguous"
    if normalized["pricing_source_kind"] != pricing.SOURCE_EXCHANGE_EXECUTION:
        return None, "unmatched"
    amount_rows = conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}, wallet_label
        FROM (
            SELECT t.*, w.label AS wallet_label
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
        )
        WHERE profile_id = ?
          AND external_id = ?
          AND direction = ?
          AND asset = ?
          AND amount = ?
          {wallet_filter}
        ORDER BY created_at DESC
        LIMIT 2
        """,
        tuple(base_params),
    ).fetchall()
    match = _single_match(amount_rows)
    if match:
        return match, "matched"
    if len(amount_rows) > 1:
        return None, "ambiguous"
    economic_match, economic_status = _find_existing_profile_transaction_by_economics(
        conn,
        profile_id,
        normalized,
        exclude_wallet_id=exclude_wallet_id,
    )
    if economic_status != "unmatched":
        return economic_match, economic_status
    return None, "unmatched"


def _find_existing_profile_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
    normalized: Mapping[str, Any],
) -> sqlite3.Row | None:
    match, _status = _find_existing_profile_transaction_result(
        conn,
        profile_id,
        normalized,
    )
    return match


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


def _raw_json_field(existing: Mapping[str, Any], field: str) -> str | None:
    raw_json = (
        existing.get("raw_json")
        if hasattr(existing, "get")
        else existing["raw_json"]
    )
    if not raw_json:
        return None
    try:
        payload = json.loads(raw_json)
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    value = payload.get(field)
    if value is None:
        return None
    return str(value)


def _metadata_field_is_import_authored(existing: Mapping[str, Any], field: str) -> bool:
    current = existing[field]
    if current in (None, ""):
        return True
    raw_value = _raw_json_field(existing, field)
    return raw_value is not None and str(current) == raw_value


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

    exchange_execution_overrides = (
        normalized["pricing_source_kind"] == pricing.SOURCE_EXCHANGE_EXECUTION
        and incoming_priority >= existing_priority
    )
    if (
        (exchange_execution_overrides and _metadata_field_is_import_authored(existing, "kind"))
        or not existing["kind"]
    ) and normalized["kind"]:
        updates["kind"] = normalized["kind"]
    if (
        (exchange_execution_overrides and _metadata_field_is_import_authored(existing, "description"))
        or not existing["description"]
    ) and normalized["description"]:
        updates["description"] = normalized["description"]
    if (
        (exchange_execution_overrides and _metadata_field_is_import_authored(existing, "counterparty"))
        or not existing["counterparty"]
    ) and normalized["counterparty"]:
        updates["counterparty"] = normalized["counterparty"]
    if not existing["payment_hash"] and normalized["payment_hash"]:
        updates["payment_hash"] = normalized["payment_hash"]
        updates["payment_hash_source"] = normalized["payment_hash_source"]
    if updates and normalized["raw_json"] and normalized["raw_json"] != existing["raw_json"]:
        updates["raw_json"] = normalized["raw_json"]
    return updates


def _import_change_record(
    tx_id: str,
    wallet_label: str,
    normalized: Mapping[str, Any],
    changed_fields: Sequence[str],
) -> dict[str, Any]:
    return {
        "transaction_id": tx_id,
        "external_id": normalized["external_id"],
        "wallet": wallet_label,
        "asset": normalized["asset"],
        "direction": normalized["direction"],
        "amount_msat": btc_to_msat(normalized["amount"]),
        "changed_fields": sorted(changed_fields),
        "pricing_external_ref": normalized["pricing_external_ref"],
    }


def normalize_bullbitcoin_import_mode(import_mode: str | None) -> str:
    mode = str(import_mode or BULLBITCOIN_IMPORT_MODE_RELEVANT).strip().lower()
    if mode in {"relevant-only", "relevant_only", "match", "match-only", "match_only"}:
        mode = BULLBITCOIN_IMPORT_MODE_RELEVANT
    if mode not in BULLBITCOIN_IMPORT_MODES:
        raise AppError(
            f"Unsupported Bull Bitcoin import mode '{import_mode}'",
            code="validation",
            hint="Choose 'relevant' to enrich matching book transactions, or 'full' to import all completed Bull orders with reconciliation flags.",
            retryable=False,
        )
    return mode


def _find_imported_wallet_transaction(
    conn: sqlite3.Connection,
    wallet_id: str,
    normalized: Mapping[str, Any],
) -> sqlite3.Row | None:
    fingerprint = make_transaction_fingerprint(
        wallet_id,
        normalized["external_id"],
        normalized["occurred_at"],
        normalized["direction"],
        normalized["asset"],
        normalized["amount"],
        normalized["fee"],
    )
    existing = _find_existing_transaction(conn, wallet_id, normalized, fingerprint)
    if existing:
        return existing
    return conn.execute(
        f"""
        SELECT {_EXISTING_TRANSACTION_COLUMNS}
        FROM transactions
        WHERE wallet_id = ?
          AND pricing_external_ref = ?
          AND direction = ?
          AND asset = ?
          AND amount = ?
        ORDER BY created_at DESC
        LIMIT 1
        """,
        (
            wallet_id,
            normalized["pricing_external_ref"],
            normalized["direction"],
            normalized["asset"],
            btc_to_msat(normalized["amount"]),
        ),
    ).fetchone()


def _apply_bullbitcoin_reconciliation_flag(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    tx_id: str,
    status: str,
    hooks: ImportCoordinatorHooks,
) -> bool:
    tag_rows: dict[str, sqlite3.Row] = {}
    for tag_status, (code, label) in BULLBITCOIN_RECONCILIATION_TAGS.items():
        tag, created = hooks.ensure_tag_row(
            conn,
            profile["workspace_id"],
            profile["id"],
            code,
            label,
        )
        tag_rows[tag_status] = tag
    conn.executemany(
        "DELETE FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?",
        [(tx_id, tag["id"]) for tag in tag_rows.values()],
    )
    conn.execute(
        "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
        (tx_id, tag_rows[status]["id"]),
    )
    # In full mode the Bull account can be shared across multiple books, so an
    # unmatched row is only a review signal until the user assigns it to this book.
    exclude = True
    conn.execute(
        "UPDATE transactions SET excluded = ? WHERE id = ?",
        (1 if exclude else 0, tx_id),
    )
    return exclude


def _apply_pocketbitcoin_reconciliation_flag(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    tx_id: str,
    status: str,
    hooks: ImportCoordinatorHooks,
) -> bool:
    tag_rows: dict[str, sqlite3.Row] = {}
    for tag_status, (code, label) in POCKETBITCOIN_RECONCILIATION_TAGS.items():
        tag, created = hooks.ensure_tag_row(
            conn,
            profile["workspace_id"],
            profile["id"],
            code,
            label,
        )
        tag_rows[tag_status] = tag
    conn.executemany(
        "DELETE FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?",
        [(tx_id, tag["id"]) for tag in tag_rows.values()],
    )
    conn.execute(
        "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
        (tx_id, tag_rows[status]["id"]),
    )
    exclude = True
    conn.execute(
        "UPDATE transactions SET excluded = ? WHERE id = ?",
        (1 if exclude else 0, tx_id),
    )
    return exclude


def _bullbitcoin_reconciliation_record(
    normalized: Mapping[str, Any],
    status: str,
    imported_tx_id: str | None,
    wallet_label: str,
    matched: sqlite3.Row | None,
) -> dict[str, Any]:
    record = _import_change_record(
        imported_tx_id or "",
        wallet_label,
        normalized,
        ("reconciliation",),
    )
    record["status"] = status
    if matched:
        record["matched_transaction_id"] = matched["id"]
        record["matched_wallet"] = matched["wallet_label"]
    return record


def _pocketbitcoin_reconciliation_record(
    normalized: Mapping[str, Any],
    status: str,
    imported_tx_id: str | None,
    wallet_label: str,
    matched: sqlite3.Row | None,
) -> dict[str, Any]:
    record = _import_change_record(
        imported_tx_id or "",
        wallet_label,
        normalized,
        ("reconciliation",),
    )
    record["status"] = status
    if matched:
        record["matched_transaction_id"] = matched["id"]
        record["matched_wallet"] = matched["wallet_label"]
    return record


def _normalized_fiat_currency(value: Any) -> str:
    return str(value or "").strip().upper()


def _import_price_currency(record: ImportRow) -> str:
    return _normalized_fiat_currency(record.get("fiat_currency"))


def _validate_import_price_currency(
    profile: Mapping[str, Any],
    normalized: Mapping[str, Any],
) -> None:
    import_currency = _normalized_fiat_currency(normalized.get("fiat_currency"))
    if not import_currency:
        return
    has_price = normalized["fiat_rate"] is not None or normalized["fiat_value"] is not None
    if not has_price:
        return
    profile_currency = _normalized_fiat_currency(profile["fiat_currency"])
    if import_currency == profile_currency:
        return
    raise AppError(
        f"Imported price currency {import_currency} does not match profile fiat currency {profile_currency}",
        code="validation",
        hint=(
            "Import the file into a profile with the same fiat currency, or remove "
            "the imported fiat price values and price the transactions separately."
        ),
        retryable=False,
    )


def _emit_import_progress(
    progress: ProgressCallback,
    *,
    wallet_label: str,
    processed: int,
    total: int,
    imported: int | None = None,
    skipped: int | None = None,
) -> None:
    payload: dict[str, Any] = {
        "phase": "importing",
        "wallet": wallet_label,
        "processed": processed,
        "total": total,
    }
    if imported is not None:
        payload["imported"] = imported
    if skipped is not None:
        payload["skipped"] = skipped
    progress(payload)


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
    payment_hash = str_or_none(record.get("payment_hash"))
    if payment_hash is not None:
        payment_hash = payment_hash.strip().lower()
        if len(payment_hash) != 64:
            payment_hash = None
        else:
            try:
                bytes.fromhex(payment_hash)
            except ValueError:
                payment_hash = None
    payment_hash_source = (
        str_or_none(record.get("payment_hash_source")) if payment_hash else None
    )
    return {
        "external_id": str(record.get("txid") or record.get("id") or ""),
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": normalize_asset_code(record.get("asset") or "BTC"),
        "amount": amount,
        "fee": fee,
        "fiat_currency": _import_price_currency(record),
        **payload,
        "kind": record.get("kind"),
        "description": record.get("description"),
        "counterparty": record.get("counterparty"),
        "payment_hash": payment_hash,
        "payment_hash_source": payment_hash_source,
        "match_without_external_id": bool(record.get("match_without_external_id")),
        "match_time_tolerance_seconds": int(record.get("match_time_tolerance_seconds") or 0),
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
    match_existing_only: bool = False,
    report_updates: bool = False,
) -> dict[str, Any]:
    """Insert parsed records and optionally enrich matching transactions.

    `updated` is a subcount of `skipped`: merge/enrichment rows do not insert a
    new transaction, so they stay in the skipped total for import accounting.
    """
    imported = 0
    skipped = 0
    updated = 0
    unchanged = 0
    inserted_records: list[dict[str, Any]] = []
    updated_records: list[dict[str, Any]] = []
    total = len(records)
    progress = sync_progress_emitter.get()
    if progress is not None:
        _emit_import_progress(
            progress,
            wallet_label=wallet["label"],
            processed=0,
            total=total,
        )
    for index, record in enumerate(records, start=1):
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
            _validate_import_price_currency(profile, normalized)
            updates = _transaction_merge_updates(existing, normalized, fingerprint)
            if updates:
                changed_fields = sorted(updates)
                assignments = ", ".join(f"{column} = ?" for column in updates)
                conn.execute(
                    f"UPDATE transactions SET {assignments} WHERE id = ?",
                    (*updates.values(), existing["id"]),
                )
                updated += 1
                updated_records.append(
                    _import_change_record(
                        existing["id"],
                        wallet["label"],
                        normalized,
                        changed_fields,
                    )
                )
            else:
                unchanged += 1
            skipped += 1
            if progress is not None and (index % 200 == 0 or index == total):
                _emit_import_progress(
                    progress,
                    wallet_label=wallet["label"],
                    processed=index,
                    total=total,
                    imported=imported,
                    skipped=skipped,
                )
            continue
        if match_existing_only:
            skipped += 1
            if progress is not None and (index % 200 == 0 or index == total):
                _emit_import_progress(
                    progress,
                    wallet_label=wallet["label"],
                    processed=index,
                    total=total,
                    imported=imported,
                    skipped=skipped,
                )
            continue
        _validate_import_price_currency(profile, normalized)
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
                counterparty, raw_json, payment_hash, payment_hash_source, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                normalized["fiat_currency"] or profile["fiat_currency"],
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
                normalized["payment_hash"],
                normalized["payment_hash_source"],
                now_iso(),
            ),
        )
        imported += 1
        inserted_records.append(
            _import_change_record(
                tx_id,
                wallet["label"],
                normalized,
                (
                    "transaction",
                    "pricing",
                    "metadata",
                ),
            )
        )
        if progress is not None and (index % 200 == 0 or index == total):
            _emit_import_progress(
                progress,
                wallet_label=wallet["label"],
                processed=index,
                total=total,
                imported=imported,
                skipped=skipped,
            )
    hooks.invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    outcome = {
        "wallet": wallet["label"],
        "source": source_label,
        "imported": imported,
        "skipped": skipped,
        "unchanged": unchanged,
        "inserted_records": inserted_records,
        "updated_records": updated_records,
    }
    if report_updates and updated:
        outcome["updated"] = updated
    return outcome


def enrich_profile_records(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    records: Sequence[ImportRow],
    source_label: str,
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
    report_updates: bool = False,
) -> dict[str, Any]:
    """Enrich existing profile transactions from exchange evidence.

    This is intentionally insert-free. Bull Bitcoin order exports describe
    exchange execution, not a wallet source of truth, so unmatched or ambiguous
    rows are left untouched.
    """
    updated = 0
    matched = 0
    unchanged = 0
    skipped_ambiguous = 0
    skipped_unmatched = 0
    skipped = 0
    updated_records: list[dict[str, Any]] = []
    total = len(records)
    progress = sync_progress_emitter.get()
    if progress is not None:
        _emit_import_progress(
            progress,
            wallet_label="book",
            processed=0,
            total=total,
        )
    for index, record in enumerate(records, start=1):
        normalized = normalize_import_record(record, source_label=source_label)
        existing, match_status = _find_existing_profile_transaction_result(
            conn,
            profile["id"],
            normalized,
        )
        if not existing:
            if match_status == "ambiguous":
                skipped_ambiguous += 1
            else:
                skipped_unmatched += 1
            skipped += 1
        else:
            matched += 1
            _validate_import_price_currency(profile, normalized)
            updates = _transaction_merge_updates(
                existing,
                normalized,
                existing["fingerprint"],
            )
            if updates:
                changed_fields = sorted(updates)
                assignments = ", ".join(f"{column} = ?" for column in updates)
                conn.execute(
                    f"UPDATE transactions SET {assignments} WHERE id = ?",
                    (*updates.values(), existing["id"]),
                )
                updated += 1
                updated_records.append(
                    _import_change_record(
                        existing["id"],
                        existing["wallet_label"],
                        normalized,
                        changed_fields,
                    )
                )
            else:
                unchanged += 1
            skipped += 1
        if progress is not None and (index % 200 == 0 or index == total):
            _emit_import_progress(
                progress,
                wallet_label="book",
                processed=index,
                total=total,
                imported=0,
                skipped=skipped,
            )
    hooks.invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    outcome = {
        "scope": "book",
        "source": source_label,
        "imported": 0,
        "skipped": skipped,
        "matched": matched,
        "unchanged": unchanged,
        "skipped_unmatched": skipped_unmatched,
        "skipped_ambiguous": skipped_ambiguous,
        "updated_records": updated_records,
    }
    if report_updates and updated:
        outcome["updated"] = updated
    return outcome


def import_bullbitcoin_records_full(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    source_label: str,
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, Any]:
    """Import every completed Bull order and flag book reconciliation status."""
    normalized_records = [
        normalize_import_record(record, source_label=source_label) for record in records
    ]
    reconciliation: list[tuple[dict[str, Any], sqlite3.Row | None, str]] = []
    matched = 0
    unmatched = 0
    ambiguous = 0
    for normalized in normalized_records:
        existing, status = _find_existing_profile_transaction_result(
            conn,
            profile["id"],
            normalized,
            exclude_wallet_id=wallet["id"],
        )
        if status == "matched":
            matched += 1
        elif status == "ambiguous":
            ambiguous += 1
        else:
            unmatched += 1
        reconciliation.append((normalized, existing, status))

    outcome = insert_wallet_records(
        conn,
        profile,
        wallet,
        records,
        source_label,
        hooks,
        commit=False,
        report_updates=True,
    )

    inserted_ids = {
        record["transaction_id"] for record in outcome.get("inserted_records", [])
    }
    updated_ids = {
        record["transaction_id"] for record in outcome.get("updated_records", [])
    }
    reconciliation_records: list[dict[str, Any]] = []
    status_by_tx_id: dict[str, dict[str, Any]] = {}
    excluded = 0
    for normalized, existing, status in reconciliation:
        imported = _find_imported_wallet_transaction(conn, wallet["id"], normalized)
        if not imported:
            continue
        row_excluded = _apply_bullbitcoin_reconciliation_flag(
            conn,
            profile,
            imported["id"],
            status,
            hooks,
        )
        if row_excluded:
            excluded += 1
        reconciliation_record = _bullbitcoin_reconciliation_record(
            normalized,
            status,
            imported["id"],
            wallet["label"],
            existing,
        )
        status_by_tx_id[imported["id"]] = {
            "status": status,
            "excluded": row_excluded,
            "matched_wallet": existing["wallet_label"] if existing else None,
            "matched_transaction_id": existing["id"] if existing else None,
        }
        if imported["id"] in inserted_ids or imported["id"] in updated_ids:
            reconciliation_record["changed"] = True
        reconciliation_records.append(reconciliation_record)

    hooks.invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    for bucket in ("inserted_records", "updated_records"):
        for record in outcome.get(bucket, []):
            record.update(status_by_tx_id.get(record["transaction_id"], {}))
    outcome.update(
        {
            "scope": "book",
            "mode": BULLBITCOIN_IMPORT_MODE_FULL,
            "matched": matched,
            "unmatched": unmatched,
            "ambiguous": ambiguous,
            "excluded": excluded,
            "reconciliation_records": reconciliation_records,
        }
    )
    return outcome


def import_pocketbitcoin_records_full(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    source_label: str,
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, Any]:
    """Import every Pocket exchange row and flag book reconciliation status."""
    normalized_records = [
        normalize_import_record(record, source_label=source_label) for record in records
    ]
    reconciliation: list[tuple[dict[str, Any], sqlite3.Row | None, str]] = []
    matched = 0
    unmatched = 0
    ambiguous = 0
    for normalized in normalized_records:
        existing, status = _find_existing_profile_transaction_result(
            conn,
            profile["id"],
            normalized,
            exclude_wallet_id=wallet["id"],
        )
        if status == "matched":
            matched += 1
        elif status == "ambiguous":
            ambiguous += 1
        else:
            unmatched += 1
        reconciliation.append((normalized, existing, status))

    outcome = insert_wallet_records(
        conn,
        profile,
        wallet,
        records,
        source_label,
        hooks,
        commit=False,
        report_updates=True,
    )

    inserted_ids = {
        record["transaction_id"] for record in outcome.get("inserted_records", [])
    }
    updated_ids = {
        record["transaction_id"] for record in outcome.get("updated_records", [])
    }
    reconciliation_records: list[dict[str, Any]] = []
    status_by_tx_id: dict[str, dict[str, Any]] = {}
    excluded = 0
    for normalized, existing, status in reconciliation:
        imported = _find_imported_wallet_transaction(conn, wallet["id"], normalized)
        if not imported:
            continue
        row_excluded = _apply_pocketbitcoin_reconciliation_flag(
            conn,
            profile,
            imported["id"],
            status,
            hooks,
        )
        if row_excluded:
            excluded += 1
        reconciliation_record = _pocketbitcoin_reconciliation_record(
            normalized,
            status,
            imported["id"],
            wallet["label"],
            existing,
        )
        status_by_tx_id[imported["id"]] = {
            "status": status,
            "excluded": row_excluded,
            "matched_wallet": existing["wallet_label"] if existing else None,
            "matched_transaction_id": existing["id"] if existing else None,
        }
        if imported["id"] in inserted_ids or imported["id"] in updated_ids:
            reconciliation_record["changed"] = True
        reconciliation_records.append(reconciliation_record)

    hooks.invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    for bucket in ("inserted_records", "updated_records"):
        for record in outcome.get(bucket, []):
            record.update(status_by_tx_id.get(record["transaction_id"], {}))
    outcome.update(
        {
            "scope": "book",
            "mode": BULLBITCOIN_IMPORT_MODE_FULL,
            "matched": matched,
            "unmatched": unmatched,
            "ambiguous": ambiguous,
            "excluded": excluded,
            "reconciliation_records": reconciliation_records,
        }
    )
    return outcome


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
    apply_river: bool = False,
    commit: bool = True,
    match_existing_only: bool = False,
    report_updates: bool = False,
) -> dict[str, Any]:
    outcome = insert_wallet_records(
        conn,
        profile,
        wallet,
        records,
        source_label,
        hooks,
        commit=False,
        match_existing_only=match_existing_only,
        report_updates=report_updates,
    )
    if apply_btcpay:
        outcome.update(apply_btcpay_metadata(conn, profile, wallet, records, hooks, commit=False))
    if apply_phoenix:
        outcome.update(apply_phoenix_metadata(conn, profile, wallet, records, hooks, commit=False))
    if apply_river:
        outcome.update(apply_river_metadata(conn, profile, wallet, records, hooks, commit=False))
    if commit:
        conn.commit()
    return outcome


def import_file_into_profile(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    file_path: str,
    input_format: str,
    hooks: ImportCoordinatorHooks,
    *,
    import_mode: str = BULLBITCOIN_IMPORT_MODE_RELEVANT,
    wallet: Mapping[str, Any] | None = None,
    commit: bool = True,
) -> dict[str, Any]:
    records = load_import_records(file_path, input_format)
    is_bull = is_bullbitcoin_format(input_format)
    is_pocket = is_pocketbitcoin_format(input_format)
    if not (is_bull or is_pocket):
        raise AppError(
            f"Profile-wide imports do not support '{input_format}'",
            code="validation",
            hint="Choose a wallet for wallet-scoped transaction imports.",
            retryable=False,
        )
    mode = normalize_bullbitcoin_import_mode(import_mode)
    if mode == BULLBITCOIN_IMPORT_MODE_FULL:
        if wallet is None:
            raise AppError(
                "A provider wallet is required for full import mode",
                code="validation",
                hint="Choose or create the book's exchange-evidence wallet.",
                retryable=False,
            )
        if is_pocket:
            outcome = import_pocketbitcoin_records_full(
                conn,
                profile,
                wallet,
                records,
                f"file:{input_format}",
                hooks,
                commit=commit,
            )
        else:
            outcome = import_bullbitcoin_records_full(
                conn,
                profile,
                wallet,
                records,
                f"file:{input_format}",
                hooks,
                commit=commit,
            )
    else:
        outcome = enrich_profile_records(
            conn,
            profile,
            records,
            f"file:{input_format}",
            hooks,
            report_updates=True,
            commit=commit,
        )
        outcome["mode"] = mode
    if is_pocket:
        outcome["pocketbitcoin_rows"] = len(records)
    else:
        outcome["bullbitcoin_rows"] = len(records)
    outcome["input_format"] = input_format
    outcome["file"] = os.path.abspath(file_path)
    return outcome


def apply_river_metadata(
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
        description = str_or_none(record.get("_river_description"))
        if description and not tx["note"]:
            conn.execute(
                "UPDATE transactions SET note = ? WHERE id = ?",
                (description, tx["id"]),
            )
            notes_set += 1
        tag_value = str_or_none(record.get("_river_tag"))
        if tag_value:
            code = f"river:{tag_value}".lower().replace(" ", "_")
            tag, created = hooks.ensure_tag_row(
                conn,
                profile["workspace_id"],
                profile["id"],
                code,
                tag_value,
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
        "river_notes_set": notes_set,
        "river_tags_added": tags_added,
        "river_tags_created": tags_created,
    }


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
        apply_river=is_river_format(input_format),
        match_existing_only=is_bullbitcoin_format(input_format) or is_pocketbitcoin_format(input_format),
        report_updates=is_bullbitcoin_format(input_format) or is_pocketbitcoin_format(input_format),
        commit=commit,
    )
    if is_bullbitcoin_format(input_format):
        outcome["bullbitcoin_rows"] = len(records)
    if is_pocketbitcoin_format(input_format):
        outcome["pocketbitcoin_rows"] = len(records)
    outcome["input_format"] = input_format
    outcome["file"] = os.path.abspath(file_path)
    return outcome


__all__ = [
    "ImportCoordinatorHooks",
    "apply_btcpay_metadata",
    "apply_phoenix_metadata",
    "apply_river_metadata",
    "enrich_profile_records",
    "import_file_into_profile",
    "normalize_bullbitcoin_import_mode",
    "import_file_into_wallet",
    "import_pocketbitcoin_records_full",
    "import_records_into_wallet",
    "insert_wallet_records",
    "is_pocketbitcoin_format",
    "make_transaction_fingerprint",
    "normalize_import_direction",
    "normalize_import_record",
]
