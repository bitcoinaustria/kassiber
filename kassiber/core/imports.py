from __future__ import annotations

"""Import orchestration helpers above the parser-only `kassiber.importers` boundary."""

import json
import os
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from types import SimpleNamespace
from typing import Any, Callable, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..fingerprints import make_transaction_fingerprint
from . import pricing
from ..importers import (
    bullbitcoin_wallet_record_network,
    exchange_evidence_label,
    exchange_evidence_rows_key,
    is_btcpay_format,
    is_bullbitcoin_format,
    is_bullbitcoin_wallet_format,
    is_coinfinity_format,
    is_exchange_evidence_format,
    is_phoenix_format,
    is_pocketbitcoin_format,
    is_river_format,
    is_strike_format,
    is_twentyonebitcoin_format,
    is_wasabi_format,
    load_wasabi_bundle,
    load_wasabi_bundle_payload,
    load_import_records,
)
from ..msat import btc_to_msat, dec
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, parse_timestamp
from ..util import str_or_none
from ..wallet_descriptors import normalize_asset_code
from . import output_inventory as core_output_inventory
from . import wallets as core_wallets
from .privacy_hops import privacy_boundary_from_import_record
from .sync import sync_progress_emitter

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
TWENTYONEBITCOIN_RECONCILIATION_TAGS = {
    "matched": ("21bitcoin-matched", "21bitcoin matched"),
    "unmatched": ("21bitcoin-wallet-gap", "21bitcoin wallet gap"),
    "ambiguous": ("21bitcoin-ambiguous", "21bitcoin ambiguous"),
}
POCKETBITCOIN_RECONCILIATION_TAGS = {
    "matched": ("pocketbitcoin-matched", "Pocket Bitcoin matched"),
    "unmatched": ("pocketbitcoin-wallet-gap", "Pocket Bitcoin wallet gap"),
    "ambiguous": ("pocketbitcoin-ambiguous", "Pocket Bitcoin ambiguous"),
}
COINFINITY_RECONCILIATION_TAGS = {
    "matched": ("coinfinity-matched", "Coinfinity matched"),
    "unmatched": ("coinfinity-wallet-gap", "Coinfinity wallet gap"),
    "ambiguous": ("coinfinity-ambiguous", "Coinfinity ambiguous"),
}
EXCHANGE_EVIDENCE_RECONCILIATION_TAGS = {
    "bullbitcoin_csv": BULLBITCOIN_RECONCILIATION_TAGS,
    "coinfinity_csv": COINFINITY_RECONCILIATION_TAGS,
    "21bitcoin_csv": TWENTYONEBITCOIN_RECONCILIATION_TAGS,
    "pocketbitcoin_csv": POCKETBITCOIN_RECONCILIATION_TAGS,
}


ProgressCallback = Callable[[Mapping[str, Any]], None]

@dataclass(frozen=True)
class ImportCoordinatorHooks:
    ensure_tag_row: EnsureTagRow
    invalidate_journals: InvalidateJournals


TRANSACTION_METADATA_CHANGE_KEYS = (
    "btcpay_notes_set",
    "btcpay_tags_added",
    "phoenix_notes_set",
    "phoenix_tags_added",
    "river_notes_set",
    "river_tags_added",
    "wasabi_notes_set",
    "wasabi_tags_added",
    "wasabi_review_marked",
    "wasabi_review_cleared",
)


def _metadata_changed(outcome: Mapping[str, Any]) -> bool:
    for key in TRANSACTION_METADATA_CHANGE_KEYS:
        value = outcome.get(key)
        try:
            if int(value or 0) > 0:
                return True
        except (TypeError, ValueError):
            continue
    return False


def _invalidate_journals_for_metadata_changes(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    outcome: dict[str, Any],
    hooks: ImportCoordinatorHooks,
) -> None:
    if outcome.get("journal_invalidated") or not _metadata_changed(outcome):
        return
    hooks.invalidate_journals(conn, profile["id"])
    outcome["journal_invalidated"] = True


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
       pricing_external_ref, pricing_quality, kind, privacy_boundary, description,
       counterparty, raw_json, payment_hash, payment_hash_source,
       swap_refund_funding_txid
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
    relax_fee_match = (
        normalized["pricing_source_kind"] == pricing.SOURCE_EXCHANGE_EXECUTION
        or normalized.get("match_existing_ignore_fee")
    )
    if existing or not relax_fee_match:
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
    if not normalized.get("exchange_evidence_match_by_economics"):
        return None, "unmatched"
    filters: list[str] = []
    params: list[Any] = [
        profile_id,
        normalized["direction"],
        normalized["asset"],
        btc_to_msat(normalized["amount"]),
    ]
    if exclude_wallet_id:
        filters.append("wallet_id != ?")
        params.append(exclude_wallet_id)
    pricing_method = normalized.get("pricing_method")
    if pricing_method:
        filters.append("NOT (excluded = 1 AND pricing_method = ?)")
        params.append(pricing_method)
    window = _timestamp_window(
        normalized["occurred_at"],
        int(normalized.get("exchange_evidence_match_time_tolerance_seconds") or 0),
    )
    if window is not None:
        filters.append("occurred_at BETWEEN ? AND ?")
        params.extend(window)
    extra_filters = "".join(f"\n          AND {condition}" for condition in filters)
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
          {extra_filters}
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
    if (
        normalized.get("exchange_evidence_match_by_economics")
        or not normalized["external_id"]
    ):
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


def _raw_json_payload(row: Mapping[str, Any]) -> Mapping[str, Any] | None:
    raw_json = row.get("raw_json") if hasattr(row, "get") else row["raw_json"]
    if not raw_json:
        return None
    try:
        payload = json.loads(raw_json)
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    return payload if isinstance(payload, Mapping) else None


def _privacy_boundary_is_import_authored(existing: Mapping[str, Any]) -> bool:
    current = existing["privacy_boundary"]
    if current in (None, ""):
        return False
    payload = _raw_json_payload(existing)
    if payload is None:
        return False
    try:
        boundary = privacy_boundary_from_import_record(payload)
    except AppError:
        return False
    return boundary == current


def _same_import_raw_source(existing: Mapping[str, Any], normalized: Mapping[str, Any]) -> bool:
    existing_payload = _raw_json_payload(existing)
    normalized_payload = _raw_json_payload(normalized)
    if existing_payload is None or normalized_payload is None:
        return False
    source = existing_payload.get("source")
    return bool(source) and source == normalized_payload.get("source")


def _same_wasabi_history_source(existing: Mapping[str, Any], normalized: Mapping[str, Any]) -> bool:
    if not _same_import_raw_source(existing, normalized):
        return False
    payload = _raw_json_payload(existing)
    return payload is not None and payload.get("source") == "wasabi_gethistory"


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
    kind_is_refreshable = (
        _same_wasabi_history_source(existing, normalized)
        and existing["kind"] in {"coinjoin", "deposit", "withdrawal"}
    )
    if (
        normalized["kind"]
        and existing["kind"] != normalized["kind"]
        and (
            (exchange_execution_overrides and _metadata_field_is_import_authored(existing, "kind"))
            or not existing["kind"]
            or kind_is_refreshable
        )
    ):
        updates["kind"] = normalized["kind"]
    if (
        normalized["privacy_boundary"]
        and existing["privacy_boundary"] != normalized["privacy_boundary"]
        and (
            not existing["privacy_boundary"]
            or _privacy_boundary_is_import_authored(existing)
        )
    ):
        updates["privacy_boundary"] = normalized["privacy_boundary"]
    elif (
        existing["privacy_boundary"]
        and normalized["privacy_boundary"] is None
        and _privacy_boundary_is_import_authored(existing)
        and _same_import_raw_source(existing, normalized)
    ):
        updates["privacy_boundary"] = None
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
    if not existing["swap_refund_funding_txid"] and normalized["swap_refund_funding_txid"]:
        updates["swap_refund_funding_txid"] = normalized["swap_refund_funding_txid"]
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
            f"Unsupported exchange-evidence import mode '{import_mode}'",
            code="validation",
            hint="Choose 'relevant' to enrich matching book transactions, or 'full' to import all provider rows with reconciliation flags.",
            retryable=False,
        )
    return mode


def filter_bullbitcoin_wallet_records(
    records: Sequence[ImportRow],
    network: str | None,
) -> list[ImportRow]:
    if not network:
        return list(records)
    normalized_network = core_wallets.normalize_bullbitcoin_wallet_network(network)
    return [
        record
        for record in records
        if bullbitcoin_wallet_record_network(record) == normalized_network
    ]


def wallet_bullbitcoin_wallet_network(wallet: Mapping[str, Any]) -> str | None:
    try:
        config = json.loads(wallet["config_json"] or "{}")
    except (KeyError, TypeError, ValueError):
        config = {}
    return core_wallets.wallet_bullbitcoin_wallet_network(config)


def _reconciliation_tags(input_format: str) -> Mapping[str, tuple[str, str]]:
    return EXCHANGE_EVIDENCE_RECONCILIATION_TAGS.get(
        input_format,
        BULLBITCOIN_RECONCILIATION_TAGS,
    )


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
    input_format: str = "bullbitcoin_csv",
) -> bool:
    tag_rows: dict[str, sqlite3.Row] = {}
    for tag_status, (code, label) in _reconciliation_tags(input_format).items():
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
    # In full mode a provider account can be shared across multiple books, so
    # an unmatched row is only a review signal until the user assigns it here.
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


def _clear_exchange_reconciliation_flags(
    conn: sqlite3.Connection,
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    source_label: str,
    input_format: str,
) -> dict[str, int]:
    tag_codes = [code for code, _label in _reconciliation_tags(input_format).values()]
    if not tag_codes:
        return {}
    placeholders = ", ".join("?" for _ in tag_codes)
    tag_rows = conn.execute(
        f"SELECT id FROM tags WHERE code IN ({placeholders})",
        tuple(tag_codes),
    ).fetchall()
    tag_ids = [tag["id"] for tag in tag_rows]
    if not tag_ids:
        return {}
    tag_placeholders = ", ".join("?" for _ in tag_ids)
    cleared = 0
    reactivated = 0
    for record in records:
        normalized = normalize_import_record(record, source_label=source_label)
        imported = _find_imported_wallet_transaction(conn, wallet["id"], normalized)
        if not imported:
            continue
        tagged = conn.execute(
            f"""
            SELECT 1
            FROM transaction_tags
            WHERE transaction_id = ?
              AND tag_id IN ({tag_placeholders})
            LIMIT 1
            """,
            (imported["id"], *tag_ids),
        ).fetchone()
        if not tagged:
            continue
        conn.execute(
            f"""
            DELETE FROM transaction_tags
            WHERE transaction_id = ?
              AND tag_id IN ({tag_placeholders})
            """,
            (imported["id"], *tag_ids),
        )
        cleared += 1
        current = conn.execute(
            "SELECT excluded FROM transactions WHERE id = ?",
            (imported["id"],),
        ).fetchone()
        if current and current["excluded"]:
            conn.execute(
                "UPDATE transactions SET excluded = 0 WHERE id = ?",
                (imported["id"],),
            )
            reactivated += 1
    outcome: dict[str, int] = {}
    if cleared:
        outcome["reconciliation_flags_cleared"] = cleared
    if reactivated:
        outcome["reactivated"] = reactivated
    return outcome


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


def _exchange_evidence_matchable(record: ImportRow) -> bool:
    return record.get("_exchange_evidence_matchable") is not False


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
    swap_refund_funding_txid = str_or_none(record.get("swap_refund_funding_txid"))
    if swap_refund_funding_txid is not None:
        swap_refund_funding_txid = swap_refund_funding_txid.strip().lower()
        if len(swap_refund_funding_txid) != 64:
            swap_refund_funding_txid = None
        else:
            try:
                bytes.fromhex(swap_refund_funding_txid)
            except ValueError:
                swap_refund_funding_txid = None
    return {
        "external_id": str(record.get("txid") or record.get("id") or ""),
        "occurred_at": occurred_at,
        "confirmed_at": confirmed_at,
        "direction": direction,
        "asset": normalize_asset_code(record.get("asset") or "BTC"),
        "amount": amount,
        "fee": fee,
        "amount_includes_fee": bool(record.get("amount_includes_fee")),
        "fiat_currency": _import_price_currency(record),
        **payload,
        "kind": record.get("kind"),
        "privacy_boundary": privacy_boundary_from_import_record(record),
        "description": record.get("description"),
        "counterparty": record.get("counterparty"),
        "payment_hash": payment_hash,
        "payment_hash_source": payment_hash_source,
        "swap_refund_funding_txid": swap_refund_funding_txid,
        "exchange_evidence_match_by_economics": bool(
            record.get("_exchange_evidence_match_by_economics")
        ),
        "exchange_evidence_match_time_tolerance_seconds": int(
            record.get("_exchange_evidence_match_time_tolerance_seconds") or 0
        ),
        "match_existing_ignore_fee": bool(record.get("_match_existing_ignore_fee")),
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
                occurred_at, confirmed_at, direction, asset, amount, fee,
                amount_includes_fee, fiat_currency,
                fiat_rate, fiat_value, fiat_price_source, fiat_rate_exact,
                fiat_value_exact, pricing_source_kind, pricing_provider, pricing_pair,
                pricing_timestamp, pricing_fetched_at, pricing_granularity,
                pricing_method, pricing_external_ref, pricing_quality, kind,
                privacy_boundary, description, counterparty, raw_json,
                payment_hash, payment_hash_source, swap_refund_funding_txid,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                1 if normalized.get("amount_includes_fee") else 0,
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
                normalized["privacy_boundary"],
                normalized["description"],
                normalized["counterparty"],
                normalized["raw_json"],
                normalized["payment_hash"],
                normalized["payment_hash_source"],
                normalized["swap_refund_funding_txid"],
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
    journal_invalidated = bool(imported or updated)
    if journal_invalidated:
        hooks.invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    outcome = {
        "wallet": wallet["label"],
        "source": source_label,
        "imported": imported,
        "skipped": skipped,
        "unchanged": unchanged,
        "journal_invalidated": journal_invalidated,
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

    This is intentionally insert-free. Exchange evidence can describe
    execution or custodial activity rather than a wallet source of truth, so
    unmatched, ambiguous, or non-wallet rows are left untouched.
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
        if not _exchange_evidence_matchable(record):
            skipped_unmatched += 1
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
            continue
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
    journal_invalidated = bool(updated)
    if journal_invalidated:
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
        "journal_invalidated": journal_invalidated,
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
    input_format: str = "bullbitcoin_csv",
    commit: bool = True,
) -> dict[str, Any]:
    """Import every provider evidence row and flag book reconciliation status."""
    normalized_records = [
        normalize_import_record(record, source_label=source_label) for record in records
    ]
    reconciliation: list[tuple[dict[str, Any], sqlite3.Row | None, str]] = []
    matched = 0
    unmatched = 0
    ambiguous = 0
    for source_record, normalized in zip(records, normalized_records):
        if _exchange_evidence_matchable(source_record):
            existing, status = _find_existing_profile_transaction_result(
                conn,
                profile["id"],
                normalized,
                exclude_wallet_id=wallet["id"],
            )
        else:
            existing, status = None, "unmatched"
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
            input_format=input_format,
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
    _invalidate_journals_for_metadata_changes(conn, profile, outcome, hooks)
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
    if not is_exchange_evidence_format(input_format):
        raise AppError(
            f"Profile-wide imports do not support '{input_format}'",
            code="validation",
            hint="Choose a wallet for wallet-scoped transaction imports.",
            retryable=False,
        )
    mode = normalize_bullbitcoin_import_mode(import_mode)
    provider_label = exchange_evidence_label(input_format)
    if mode == BULLBITCOIN_IMPORT_MODE_FULL:
        if wallet is None:
            raise AppError(
                f"A {provider_label} wallet is required for full import mode",
                code="validation",
                hint=f"Choose or create the book's {provider_label} wallet.",
                retryable=False,
            )
        outcome = import_bullbitcoin_records_full(
            conn,
            profile,
            wallet,
            records,
            f"file:{input_format}",
            hooks,
            input_format=input_format,
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
    outcome[exchange_evidence_rows_key(input_format)] = len(records)
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


def apply_wasabi_metadata(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    records: Sequence[ImportRow],
    metadata: Mapping[str, Any],
    hooks: ImportCoordinatorHooks,
    *,
    commit: bool = True,
) -> dict[str, int]:
    notes_set = 0
    tags_added = 0
    tags_created = 0
    review_marked = 0
    review_cleared = 0
    wasabi_tag, created = hooks.ensure_tag_row(
        conn,
        profile["workspace_id"],
        profile["id"],
        "wasabi",
        "Wasabi",
    )
    if created:
        tags_created += 1
    coinjoin_tag, created = hooks.ensure_tag_row(
        conn,
        profile["workspace_id"],
        profile["id"],
        "coinjoin",
        "CoinJoin",
    )
    if created:
        tags_created += 1
    privacy_tag, created = hooks.ensure_tag_row(
        conn,
        profile["workspace_id"],
        profile["id"],
        "privacy-hop-review",
        "Privacy hop review",
    )
    if created:
        tags_created += 1
    for record in records:
        txid = str_or_none(record.get("txid"))
        if not txid:
            continue
        tx = conn.execute(
            """
            SELECT id, note, review_status, privacy_boundary
            FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (profile["id"], wallet["id"], txid),
        ).fetchone()
        if not tx:
            continue
        label = str_or_none(record.get("_wasabi_label"))
        if label and not tx["note"]:
            conn.execute("UPDATE transactions SET note = ? WHERE id = ?", (label, tx["id"]))
            notes_set += 1
        tag_ids = [wasabi_tag["id"]]
        if record.get("_wasabi_islikelycoinjoin"):
            tag_ids.extend([coinjoin_tag["id"], privacy_tag["id"]])
            if tx["review_status"] not in {"review", "completed"}:
                conn.execute(
                    "UPDATE transactions SET review_status = ? WHERE id = ?",
                    ("review", tx["id"]),
                )
                review_marked += 1
        elif not tx["privacy_boundary"]:
            had_privacy_review_tag = conn.execute(
                """
                SELECT 1
                FROM transaction_tags
                WHERE transaction_id = ? AND tag_id = ?
                LIMIT 1
                """,
                (tx["id"], privacy_tag["id"]),
            ).fetchone()
            conn.execute(
                """
                DELETE FROM transaction_tags
                WHERE transaction_id = ?
                  AND tag_id IN (?, ?)
                """,
                (tx["id"], coinjoin_tag["id"], privacy_tag["id"]),
            )
            if had_privacy_review_tag and tx["review_status"] == "review":
                conn.execute(
                    "UPDATE transactions SET review_status = NULL WHERE id = ?",
                    (tx["id"],),
                )
                review_cleared += 1
        for tag_id in tag_ids:
            before = conn.total_changes
            conn.execute(
                "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                (tx["id"], tag_id),
            )
            if conn.total_changes > before:
                tags_added += 1
    try:
        config = json.loads(wallet["config_json"] or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        config = {}
    original_config = dict(config)
    if metadata:
        config["wasabi_metadata"] = json_ready(metadata)
    config.setdefault("chain", "bitcoin")
    config.setdefault("network", "mainnet")
    config.setdefault("source_format", "wasabi_bundle")
    if metadata or config != original_config:
        conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps(config, sort_keys=True), wallet["id"]),
        )
    if commit:
        conn.commit()
    return {
        "wasabi_notes_set": notes_set,
        "wasabi_tags_added": tags_added,
        "wasabi_tags_created": tags_created,
        "wasabi_review_marked": review_marked,
        "wasabi_review_cleared": review_cleared,
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
    if is_wasabi_format(input_format):
        outcome = import_wasabi_bundle_into_wallet(
            conn,
            profile,
            wallet,
            load_wasabi_bundle(file_path),
            hooks,
            source_label=f"file:{input_format}",
            commit=False,
        )
        outcome["file"] = os.path.abspath(file_path)
        if commit:
            conn.commit()
        return outcome
    records = load_import_records(file_path, input_format)
    bullbitcoin_wallet_rows_total = len(records) if is_bullbitcoin_wallet_format(input_format) else None
    bullbitcoin_wallet_network = None
    if is_bullbitcoin_wallet_format(input_format):
        bullbitcoin_wallet_network = wallet_bullbitcoin_wallet_network(wallet)
        records = filter_bullbitcoin_wallet_records(records, bullbitcoin_wallet_network)
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
        match_existing_only=(
            is_bullbitcoin_format(input_format)
            or is_coinfinity_format(input_format)
            or is_pocketbitcoin_format(input_format)
        ),
        report_updates=is_exchange_evidence_format(input_format) or is_strike_format(input_format),
        commit=False,
    )
    if is_twentyonebitcoin_format(input_format):
        outcome.update(
            _clear_exchange_reconciliation_flags(
                conn,
                wallet,
                records,
                f"file:{input_format}",
                input_format,
            )
        )
    if commit:
        conn.commit()
    if is_exchange_evidence_format(input_format):
        outcome[exchange_evidence_rows_key(input_format)] = len(records)
    if is_strike_format(input_format):
        outcome["strike_rows"] = len(records)
    if is_bullbitcoin_wallet_format(input_format):
        outcome["bullbitcoin_wallet_rows"] = len(records)
        outcome["bullbitcoin_wallet_rows_total"] = bullbitcoin_wallet_rows_total
        if bullbitcoin_wallet_network:
            outcome["bullbitcoin_wallet_network"] = bullbitcoin_wallet_network
    outcome["input_format"] = input_format
    outcome["file"] = os.path.abspath(file_path)
    return outcome


def import_wasabi_bundle_payload_into_wallet(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    payload: Any,
    hooks: ImportCoordinatorHooks,
    *,
    source_label: str = "inline:wasabi_bundle",
    commit: bool = True,
) -> dict[str, Any]:
    return import_wasabi_bundle_into_wallet(
        conn,
        profile,
        wallet,
        load_wasabi_bundle_payload(payload),
        hooks,
        source_label=source_label,
        commit=commit,
    )


def import_wasabi_bundle_into_wallet(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    bundle: Mapping[str, Any],
    hooks: ImportCoordinatorHooks,
    *,
    source_label: str,
    commit: bool = True,
) -> dict[str, Any]:
    records = list(bundle.get("records") or [])
    outcome = import_records_into_wallet(
        conn,
        profile,
        wallet,
        records,
        source_label,
        hooks,
        commit=False,
    )
    outcome.update(
        apply_wasabi_metadata(
            conn,
            profile,
            wallet,
            records,
            bundle.get("metadata") or {},
            hooks,
            commit=False,
        )
    )
    coins = bundle.get("coins") or []
    if bundle.get("coin_sections_present"):
        config = json.loads(wallet["config_json"] or "{}")
        chain = str(config.get("chain") or "bitcoin")
        network = str(config.get("network") or "mainnet")
        inventory = core_output_inventory.update_wallet_output_inventory(
            conn,
            profile,
            wallet,
            {"name": "wasabi", "kind": "wasabi_bundle"},
            SimpleNamespace(chain=chain, network=network),
            coins,
            commit=False,
        )
        outcome.update(
            {
                "wasabi_coins_observed": inventory["observed"],
                "wasabi_coins_active": inventory["active"],
                "wasabi_coins_marked_spent": inventory["spent"],
            }
        )
    outcome.update(
        {
            "wasabi_transactions": len(records),
            "wasabi_payments_in_coinjoin": len(bundle.get("payments_in_coinjoin") or []),
            "wasabi_wallet_json_present": bool(bundle.get("wallet_json_present")),
            "wasabi_listkeys_count": int(bundle.get("listkeys_count") or 0),
            "input_format": "wasabi_bundle",
        }
    )
    _invalidate_journals_for_metadata_changes(conn, profile, outcome, hooks)
    if commit:
        conn.commit()
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
    "import_wasabi_bundle_payload_into_wallet",
    "import_records_into_wallet",
    "insert_wallet_records",
    "make_transaction_fingerprint",
    "normalize_import_direction",
    "normalize_import_record",
]
