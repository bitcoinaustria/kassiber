from __future__ import annotations

import base64
import binascii
import json
import sqlite3
import uuid
from typing import Any, Mapping, Sequence

from ..errors import AppError
from ..msat import msat_to_btc
from ..redaction import redact_secret_value


DEFAULT_HISTORY_LIMIT = 50
MAX_HISTORY_LIMIT = 1000
MAX_EDIT_REASON_CHARS = 1000
EDIT_SOURCES = {"cli", "gui", "ai_tool"}

PRICING_FIELDS = {
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
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
}
TAX_FIELDS = {"review_status", "taxable", "at_regime", "at_category"}
METADATA_FIELDS = {"note", "tags", "excluded"}
SUPPORTED_FIELDS = METADATA_FIELDS | TAX_FIELDS | PRICING_FIELDS

FIELD_FAMILIES = {
    **{field: "metadata" for field in METADATA_FIELDS},
    **{field: "tax" for field in TAX_FIELDS},
    **{field: "pricing" for field in PRICING_FIELDS},
}
FIELD_FAMILY_VALUES = {"metadata", "tax", "pricing"}

FIELD_LABELS = {
    "note": "Note",
    "tags": "Tags",
    "excluded": "Report inclusion",
    "review_status": "Review status",
    "taxable": "Taxable",
    "at_regime": "Austrian regime",
    "at_category": "Austrian category",
    "fiat_currency": "Fiat currency",
    "fiat_rate": "Price per BTC",
    "fiat_value": "Fiat value",
    "fiat_price_source": "Legacy price source",
    "pricing_source_kind": "Pricing source",
    "pricing_provider": "Pricing provider",
    "pricing_pair": "Pricing pair",
    "pricing_timestamp": "Pricing timestamp",
    "pricing_fetched_at": "Pricing captured at",
    "pricing_granularity": "Pricing granularity",
    "pricing_method": "Pricing method",
    "pricing_external_ref": "Pricing evidence reference",
    "pricing_quality": "Pricing quality",
}

SOURCE_LABELS = {
    "cli": "CLI",
    "gui": "Desktop",
    "ai_tool": "Assistant",
}

REVIEW_STATUS_LABELS = {
    "completed": "Completed",
    "pending": "Pending",
    "failed": "Failed",
    "review": "Needs review",
}

AT_REGIME_LABELS = {
    "alt": "Altbestand",
    "neu": "Neubestand",
    "outside": "Outside §27b",
}

AT_CATEGORY_LABELS = {
    "income_general": "General income",
    "income_capital_yield": "Capital yield",
    "neu_gain": "Neubestand gain",
    "neu_loss": "Neubestand loss",
    "neu_swap": "Neubestand swap",
    "alt_spekulation": "Altbestand speculation",
    "alt_taxfree": "Altbestand tax-free",
    "none": "Not reportable",
}

PRICING_SOURCE_LABELS = {
    "generic_import": "Generic import",
    "wallet_export": "Wallet export",
    "exchange_execution": "Exchange execution",
    "btcpay_wallet_export": "BTCPay wallet export",
    "btcpay_invoice": "BTCPay invoice",
    "btcpay_payment": "BTCPay payment",
    "manual_override": "Manual override",
    "manual_rate_cache": "Manual rate cache",
    "fmv_provider": "Provider fair-market price",
}

PRICING_QUALITY_LABELS = {
    "exact": "Exact",
    "provider_sample": "Provider sample",
    "coarse_fallback": "Coarse fallback",
    "missing": "Missing",
}

_FIELD_SORT_ORDER = [
    "note",
    "tags",
    "excluded",
    "review_status",
    "taxable",
    "at_regime",
    "at_category",
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "pricing_source_kind",
    "pricing_quality",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_fetched_at",
]
_FIELD_SORT_INDEX = {field: index for index, field in enumerate(_FIELD_SORT_ORDER)}


def normalize_source(source: str | None) -> str:
    value = (source or "cli").strip().lower()
    if value not in EDIT_SOURCES:
        raise AppError(
            "transaction edit source is not supported",
            code="validation",
            details={"source": source, "supported": sorted(EDIT_SOURCES)},
            retryable=False,
        )
    return value


def clean_reason(reason: str | None) -> str | None:
    if reason is None:
        return None
    if not isinstance(reason, str):
        raise AppError("reason must be a string or null", code="validation", retryable=False)
    cleaned = reason.strip()
    if not cleaned:
        return None
    if len(cleaned) > MAX_EDIT_REASON_CHARS:
        raise AppError(
            f"reason cannot exceed {MAX_EDIT_REASON_CHARS} characters",
            code="validation",
            retryable=False,
        )
    return cleaned


def _row_get(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    try:
        if hasattr(row, "keys") and key not in row.keys():
            return default
        return row[key]
    except (IndexError, KeyError):
        return default


def _exact_or_float(row: Mapping[str, Any], exact_key: str, legacy_key: str) -> str | None:
    exact = _row_get(row, exact_key)
    if exact not in (None, ""):
        return str(exact)
    legacy = _row_get(row, legacy_key)
    if legacy is None:
        return None
    return format(float(legacy), ".12g")


def _canonical_tags(tags: Sequence[Mapping[str, Any] | str] | None) -> list[str]:
    labels: list[str] = []
    seen: set[str] = set()
    for item in tags or []:
        label = item.get("label") if isinstance(item, Mapping) else item
        text = str(label or "").strip()
        if not text:
            continue
        key = text.lower()
        if key in seen:
            continue
        seen.add(key)
        labels.append(text)
    return sorted(labels, key=lambda value: value.lower())


def transaction_state(
    tx: Mapping[str, Any],
    tags: Sequence[Mapping[str, Any] | str] | None,
) -> dict[str, Any]:
    return {
        "note": _row_get(tx, "note"),
        "tags": _canonical_tags(tags),
        "excluded": bool(_row_get(tx, "excluded", 0)),
        "review_status": _row_get(tx, "review_status"),
        "taxable": None
        if _row_get(tx, "taxability_override") is None
        else bool(_row_get(tx, "taxability_override")),
        "at_regime": _row_get(tx, "at_regime_override"),
        "at_category": _row_get(tx, "at_category_override"),
        "fiat_currency": _row_get(tx, "fiat_currency"),
        "fiat_rate": _exact_or_float(tx, "fiat_rate_exact", "fiat_rate"),
        "fiat_value": _exact_or_float(tx, "fiat_value_exact", "fiat_value"),
        "fiat_price_source": _row_get(tx, "fiat_price_source"),
        "pricing_source_kind": _row_get(tx, "pricing_source_kind"),
        "pricing_provider": _row_get(tx, "pricing_provider"),
        "pricing_pair": _row_get(tx, "pricing_pair"),
        "pricing_timestamp": _row_get(tx, "pricing_timestamp"),
        "pricing_fetched_at": _row_get(tx, "pricing_fetched_at"),
        "pricing_granularity": _row_get(tx, "pricing_granularity"),
        "pricing_method": _row_get(tx, "pricing_method"),
        "pricing_external_ref": _row_get(tx, "pricing_external_ref"),
        "pricing_quality": _row_get(tx, "pricing_quality"),
    }


def value_for_tx_update(field: str, value: Any) -> Any:
    if field in {"excluded", "taxable"}:
        if value is None:
            return None
        return bool(value)
    if field == "tags":
        return _canonical_tags(value)
    if field in {"fiat_rate", "fiat_value"} and value is not None:
        return str(value)
    return value


def values_differ(before: Any, after: Any) -> bool:
    return _json_value(before) != _json_value(after)


def _json_value(value: Any) -> str:
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True)


def _load_json_value(value: str | None) -> Any:
    if value is None:
        return None
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        return value


def _redacted_value(value: Any) -> tuple[Any, bool]:
    redacted = redact_secret_value(value)
    return redacted, _json_value(redacted) != _json_value(value)


def _label_for_value(field: str, value: Any) -> str:
    if value is None:
        return "Empty"
    if field == "tags":
        if not value:
            return "No tags"
        return ", ".join(str(item) for item in value)
    if field == "excluded":
        return "Excluded from reports" if value else "Included in reports"
    if field == "taxable":
        return "Taxable" if value else "Not taxable"
    if field == "review_status":
        return REVIEW_STATUS_LABELS.get(str(value), str(value))
    if field == "at_regime":
        return AT_REGIME_LABELS.get(str(value), str(value))
    if field == "at_category":
        return AT_CATEGORY_LABELS.get(str(value), str(value))
    if field == "pricing_source_kind":
        return PRICING_SOURCE_LABELS.get(str(value), str(value))
    if field == "pricing_quality":
        return PRICING_QUALITY_LABELS.get(str(value), str(value))
    if isinstance(value, bool):
        return "Yes" if value else "No"
    return str(value)


def _field_diff(field: str, before: Any, after: Any) -> dict[str, Any]:
    if field == "tags":
        before_set = {str(item).lower(): str(item) for item in before or []}
        after_set = {str(item).lower(): str(item) for item in after or []}
        added_keys = sorted(set(after_set) - set(before_set))
        removed_keys = sorted(set(before_set) - set(after_set))
        return {
            "added": [after_set[key] for key in added_keys],
            "removed": [before_set[key] for key in removed_keys],
            "before": before or [],
            "after": after or [],
        }
    return {}


def append_event(
    conn: sqlite3.Connection,
    *,
    workspace: Mapping[str, Any],
    profile: Mapping[str, Any],
    tx: Mapping[str, Any],
    source: str,
    reason: str | None,
    changed_at: str,
    changed_fields: Sequence[str],
    before_state: Mapping[str, Any],
    after_state: Mapping[str, Any],
) -> str:
    event_id = str(uuid.uuid4())
    journal_input_version = int(_row_get(profile, "journal_input_version", 0) or 0)
    conn.execute(
        """
        INSERT INTO transaction_edit_events(
            id, workspace_id, profile_id, transaction_id, wallet_id,
            transaction_external_id, transaction_occurred_at, source, reason,
            changed_at, journal_input_version, journal_input_version_after,
            last_processed_input_version, last_processed_at, last_processed_tx_count
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            event_id,
            workspace["id"],
            profile["id"],
            tx["id"],
            _row_get(tx, "wallet_id"),
            _row_get(tx, "external_id"),
            _row_get(tx, "occurred_at"),
            normalize_source(source),
            clean_reason(reason),
            changed_at,
            journal_input_version,
            journal_input_version + 1,
            int(_row_get(profile, "last_processed_input_version", 0) or 0),
            _row_get(profile, "last_processed_at"),
            int(_row_get(profile, "last_processed_tx_count", 0) or 0),
        ),
    )
    field_rows = []
    for field in sorted(changed_fields, key=lambda item: (_FIELD_SORT_INDEX.get(item, 999), item)):
        before = before_state.get(field)
        after = after_state.get(field)
        field_rows.append(
            (
                str(uuid.uuid4()),
                event_id,
                field,
                _json_value(before),
                _json_value(after),
                _json_value(_field_diff(field, before, after)),
            )
        )
    conn.executemany(
        """
        INSERT INTO transaction_edit_fields(
            id, event_id, field, before_value, after_value, diff_json
        ) VALUES(?, ?, ?, ?, ?, ?)
        """,
        field_rows,
    )
    return event_id


def _encode_cursor(row: Mapping[str, Any], filters: Mapping[str, Any]) -> str:
    token = json.dumps(
        {"changed_at": row["changed_at"], "filters": filters, "id": row["id"]},
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_cursor(cursor: str | None, filters: Mapping[str, Any]) -> Mapping[str, str] | None:
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        if not payload.get("changed_at") or not payload.get("id"):
            raise ValueError("missing cursor fields")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        ) from exc


def _effective_limit(limit: int | None) -> int:
    value = DEFAULT_HISTORY_LIMIT if limit is None else int(limit)
    if value <= 0:
        raise AppError("--limit must be positive", code="validation", retryable=False)
    if value > MAX_HISTORY_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_HISTORY_LIMIT}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_HISTORY_LIMIT}.",
            retryable=False,
        )
    return value


def _fields_for_family(field_family: str | None) -> set[str] | None:
    if field_family is None:
        return None
    family = field_family.strip().lower()
    if family not in FIELD_FAMILY_VALUES:
        raise AppError(
            "field_family is not supported",
            code="validation",
            details={"field_family": field_family, "supported": sorted(FIELD_FAMILY_VALUES)},
            retryable=False,
        )
    return {field for field, value in FIELD_FAMILIES.items() if value == family}


def _field_rows_for_events(
    conn: sqlite3.Connection,
    event_ids: Sequence[str],
) -> dict[str, list[sqlite3.Row]]:
    if not event_ids:
        return {}
    placeholders = ",".join("?" for _ in event_ids)
    rows = conn.execute(
        f"""
        SELECT *
        FROM transaction_edit_fields
        WHERE event_id IN ({placeholders})
        ORDER BY field ASC, id ASC
        """,
        tuple(event_ids),
    ).fetchall()
    grouped: dict[str, list[sqlite3.Row]] = {}
    for row in rows:
        grouped.setdefault(row["event_id"], []).append(row)
    return grouped


def _field_payload(row: Mapping[str, Any]) -> dict[str, Any]:
    field = row["field"]
    before = _load_json_value(row["before_value"])
    after = _load_json_value(row["after_value"])
    before_redacted, before_changed = _redacted_value(before)
    after_redacted, after_changed = _redacted_value(after)
    diff = _load_json_value(row["diff_json"]) or {}
    redacted_diff, diff_changed = _redacted_value(diff)
    return {
        "id": row["id"],
        "field": field,
        "label": FIELD_LABELS.get(field, field),
        "family": FIELD_FAMILIES.get(field, "metadata"),
        "before_value": before_redacted,
        "after_value": after_redacted,
        "before_label": _label_for_value(field, before_redacted),
        "after_label": _label_for_value(field, after_redacted),
        "diff": redacted_diff,
        "redacted": before_changed or after_changed or diff_changed,
    }


def _event_summary(fields: Sequence[Mapping[str, Any]]) -> str:
    field_names = {field["field"] for field in fields}
    if field_names & PRICING_FIELDS:
        return "Pricing provenance updated"
    if "tags" in field_names:
        tag_field = next((field for field in fields if field["field"] == "tags"), None)
        diff = tag_field.get("diff", {}) if tag_field else {}
        added = diff.get("added") if isinstance(diff, Mapping) else []
        removed = diff.get("removed") if isinstance(diff, Mapping) else []
        parts = []
        if added:
            parts.append(f"added {', '.join(str(item) for item in added)}")
        if removed:
            parts.append(f"removed {', '.join(str(item) for item in removed)}")
        return "Tags updated" + (f": {'; '.join(parts)}" if parts else "")
    if "note" in field_names and len(field_names) == 1:
        return "Note updated"
    if "excluded" in field_names and len(field_names) == 1:
        field = next(field for field in fields if field["field"] == "excluded")
        return field["after_label"]
    labels = [FIELD_LABELS.get(field["field"], field["field"]) for field in fields]
    return "Updated " + ", ".join(labels)


def _event_payload(
    row: Mapping[str, Any],
    fields: Sequence[Mapping[str, Any]],
    *,
    current_processed_input_version: int,
) -> dict[str, Any]:
    field_payloads = [_field_payload(field) for field in fields]
    families = sorted({field["family"] for field in field_payloads})
    amount = _row_get(row, "amount")
    fee = _row_get(row, "fee")
    return {
        "id": row["id"],
        "transaction_id": row["transaction_id"],
        "transaction_external_id": _row_get(row, "transaction_external_id") or "",
        "transaction_occurred_at": _row_get(row, "transaction_occurred_at") or "",
        "wallet_id": _row_get(row, "wallet_id") or "",
        "wallet_label": _row_get(row, "wallet_label") or "",
        "source": row["source"],
        "source_label": SOURCE_LABELS.get(row["source"], row["source"]),
        "reason": row["reason"] or "",
        "changed_at": row["changed_at"],
        "summary": _event_summary(field_payloads),
        "families": families,
        "fields": field_payloads,
        "report_anchor": {
            "journal_input_version": int(row["journal_input_version"] or 0),
            "journal_input_version_after": int(row["journal_input_version_after"] or 0),
            "last_processed_input_version": int(row["last_processed_input_version"] or 0),
            "last_processed_at": row["last_processed_at"],
            "last_processed_tx_count": int(row["last_processed_tx_count"] or 0),
            "stale_for_reports": int(row["journal_input_version_after"] or 0)
            > current_processed_input_version,
        },
        "transaction": {
            "id": row["transaction_id"],
            "external_id": _row_get(row, "transaction_external_id") or "",
            "occurred_at": _row_get(row, "transaction_occurred_at") or "",
            "direction": _row_get(row, "direction") or "",
            "asset": _row_get(row, "asset") or "",
            "amount": float(msat_to_btc(amount)) if amount is not None else None,
            "amount_msat": amount,
            "fee": float(msat_to_btc(fee)) if fee is not None else None,
            "fee_msat": fee,
            "counterparty": _row_get(row, "counterparty") or "",
        },
    }


def list_history(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: Any,
    *,
    transaction_ref: str | None = None,
    wallet_ref: str | None = None,
    source: str | None = None,
    field_family: str | None = None,
    field: str | None = None,
    pricing_only: bool = False,
    ai_only: bool = False,
    stale_only: bool = False,
    start: str | None = None,
    end: str | None = None,
    cursor: str | None = None,
    limit: int | None = None,
    include_stale: bool = True,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = _effective_limit(limit)
    current_processed_input_version = int(_row_get(profile, "last_processed_input_version", 0) or 0)
    where = ["e.profile_id = ?"]
    params: list[Any] = [profile["id"]]

    tx_id = ""
    wallet_id = ""
    clean_source = ""
    clean_field = ""
    clean_family = field_family.strip().lower() if isinstance(field_family, str) else ""
    start_ts = hooks.iso_z(hooks.parse_iso_datetime(start, "start")) if start else ""
    end_ts = hooks.iso_z(hooks.parse_iso_datetime(end, "end")) if end else ""

    if transaction_ref:
        tx = hooks.resolve_transaction(conn, profile["id"], transaction_ref)
        tx_id = tx["id"]
        where.append("e.transaction_id = ?")
        params.append(tx_id)
    if wallet_ref:
        wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_id = wallet["id"]
        where.append("e.wallet_id = ?")
        params.append(wallet_id)
    if ai_only:
        source = "ai_tool"
    if source:
        clean_source = normalize_source(source)
        where.append("e.source = ?")
        params.append(clean_source)
    family_fields = _fields_for_family("pricing" if pricing_only else clean_family or None)
    if field:
        clean_field = field.strip()
        if clean_field not in SUPPORTED_FIELDS:
            raise AppError(
                "history field is not supported",
                code="validation",
                details={"field": field, "supported": sorted(SUPPORTED_FIELDS)},
                retryable=False,
            )
        family_fields = {clean_field}
    if family_fields:
        placeholders = ",".join("?" for _ in family_fields)
        where.append(
            "EXISTS (SELECT 1 FROM transaction_edit_fields f "
            f"WHERE f.event_id = e.id AND f.field IN ({placeholders}))"
        )
        params.extend(sorted(family_fields))
    if stale_only:
        where.append("e.journal_input_version_after > ?")
        params.append(current_processed_input_version)
    if start_ts:
        where.append("e.changed_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("e.changed_at <= ?")
        params.append(end_ts)

    cursor_filters = {
        "workspace_id": workspace["id"],
        "profile_id": profile["id"],
        "transaction_id": tx_id,
        "wallet_id": wallet_id,
        "source": clean_source,
        "field_family": clean_family,
        "field": clean_field,
        "pricing_only": bool(pricing_only),
        "ai_only": bool(ai_only),
        "stale_only": bool(stale_only),
        "start": start_ts,
        "end": end_ts,
    }
    cursor_data = _decode_cursor(cursor, cursor_filters)
    if cursor_data:
        where.append("(e.changed_at < ? OR (e.changed_at = ? AND e.id < ?))")
        params.extend([cursor_data["changed_at"], cursor_data["changed_at"], cursor_data["id"]])

    params.append(effective_limit + 1)
    rows = conn.execute(
        f"""
        SELECT
            e.*,
            w.label AS wallet_label,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.counterparty
        FROM transaction_edit_events e
        LEFT JOIN wallets w ON w.id = e.wallet_id
        LEFT JOIN transactions t ON t.id = e.transaction_id
        WHERE {' AND '.join(where)}
        ORDER BY e.changed_at DESC, e.id DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    fields_by_event = _field_rows_for_events(conn, [row["id"] for row in page])
    events = [
        _event_payload(
            row,
            fields_by_event.get(row["id"], []),
            current_processed_input_version=current_processed_input_version,
        )
        for row in page
    ]
    payload = {
        "events": events,
        "next_cursor": _encode_cursor(page[-1], cursor_filters) if has_more and page else None,
        "has_more": has_more,
        "limit": effective_limit,
        "filters": {
            "transaction_id": tx_id,
            "wallet_id": wallet_id,
            "source": clean_source,
            "field_family": clean_family,
            "field": clean_field,
            "pricing_only": bool(pricing_only),
            "ai_only": bool(ai_only),
            "stale_only": bool(stale_only),
            "start": start_ts,
            "end": end_ts,
        },
    }
    if include_stale:
        payload["stale"] = stale_summary(conn, profile)
    return payload


def stale_summary(conn: sqlite3.Connection, profile: Mapping[str, Any]) -> dict[str, Any]:
    processed_version = int(_row_get(profile, "last_processed_input_version", 0) or 0)
    summary_row = conn.execute(
        """
        SELECT COUNT(*) AS count, MAX(changed_at) AS latest
        FROM transaction_edit_events
        WHERE profile_id = ? AND journal_input_version_after > ?
        """,
        (profile["id"], processed_version),
    ).fetchone()
    field_rows = conn.execute(
        """
        SELECT f.field, COUNT(*) AS count
        FROM transaction_edit_events e
        JOIN transaction_edit_fields f ON f.event_id = e.id
        WHERE e.profile_id = ? AND e.journal_input_version_after > ?
        GROUP BY f.field
        """,
        (profile["id"], processed_version),
    ).fetchall()
    source_rows = conn.execute(
        """
        SELECT source, COUNT(*) AS count
        FROM transaction_edit_events
        WHERE profile_id = ? AND journal_input_version_after > ?
        GROUP BY source
        """,
        (profile["id"], processed_version),
    ).fetchall()
    source_counts: dict[str, int] = {}
    family_counts: dict[str, int] = {}
    field_counts: dict[str, int] = {}
    event_count = int(summary_row["count"] or 0) if summary_row else 0
    latest = summary_row["latest"] if summary_row and summary_row["latest"] else None
    for row in source_rows:
        source_counts[row["source"]] = int(row["count"] or 0)
    for row in field_rows:
        count = int(row["count"] or 0)
        field = row["field"]
        family = FIELD_FAMILIES.get(field, "metadata")
        family_counts[family] = family_counts.get(family, 0) + count
        field_counts[field] = field_counts.get(field, 0) + count
    return {
        "edit_count": event_count,
        "latest_changed_at": latest,
        "source_counts": dict(sorted(source_counts.items())),
        "family_counts": dict(sorted(family_counts.items())),
        "field_counts": dict(sorted(field_counts.items())),
        "last_processed_input_version": processed_version,
        "last_processed_at": _row_get(profile, "last_processed_at"),
    }


def load_event_for_revert(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transaction_id: str,
    event_id: str,
) -> dict[str, Any]:
    event = conn.execute(
        """
        SELECT *
        FROM transaction_edit_events
        WHERE profile_id = ? AND transaction_id = ? AND id = ?
        """,
        (profile_id, transaction_id, event_id),
    ).fetchone()
    if not event:
        raise AppError(
            "transaction edit event not found",
            code="not_found",
            hint="Use the event id from transaction history.",
            retryable=False,
        )
    fields = conn.execute(
        """
        SELECT *
        FROM transaction_edit_fields
        WHERE event_id = ?
        ORDER BY field ASC, id ASC
        """,
        (event_id,),
    ).fetchall()
    return {
        "event": event,
        "fields": [
            {
                "field": row["field"],
                "before_value": _load_json_value(row["before_value"]),
                "after_value": _load_json_value(row["after_value"]),
            }
            for row in fields
        ],
    }


def history_for_transaction_ids(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    transaction_ids: Sequence[str],
) -> dict[str, list[dict[str, Any]]]:
    unique_ids = sorted(set(transaction_ids))
    if not unique_ids:
        return {}
    placeholders = ",".join("?" for _ in unique_ids)
    rows = conn.execute(
        f"""
        SELECT
            e.*,
            w.label AS wallet_label,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.counterparty
        FROM transaction_edit_events e
        LEFT JOIN wallets w ON w.id = e.wallet_id
        LEFT JOIN transactions t ON t.id = e.transaction_id
        WHERE e.profile_id = ? AND e.transaction_id IN ({placeholders})
        ORDER BY e.changed_at ASC, e.id ASC
        """,
        (profile["id"], *unique_ids),
    ).fetchall()
    fields_by_event = _field_rows_for_events(conn, [row["id"] for row in rows])
    processed_version = int(_row_get(profile, "last_processed_input_version", 0) or 0)
    grouped: dict[str, list[dict[str, Any]]] = {tx_id: [] for tx_id in unique_ids}
    for row in rows:
        grouped.setdefault(row["transaction_id"], []).append(
            _event_payload(
                row,
                fields_by_event.get(row["id"], []),
                current_processed_input_version=processed_version,
            )
        )
    return grouped


__all__ = [
    "DEFAULT_HISTORY_LIMIT",
    "EDIT_SOURCES",
    "FIELD_FAMILY_VALUES",
    "MAX_HISTORY_LIMIT",
    "PRICING_FIELDS",
    "SUPPORTED_FIELDS",
    "append_event",
    "clean_reason",
    "history_for_transaction_ids",
    "list_history",
    "load_event_for_revert",
    "normalize_source",
    "stale_summary",
    "transaction_state",
    "value_for_tx_update",
    "values_differ",
]
