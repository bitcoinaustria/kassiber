from __future__ import annotations

import base64
import binascii
import json
import os
import sqlite3
import uuid
from dataclasses import dataclass
from typing import Any, Callable, Mapping, Sequence

from ..errors import AppError
from ..importers import load_bip329_file
from ..msat import dec, msat_to_btc
from . import pricing
from . import ownership
from . import transaction_history

DEFAULT_RECORDS_LIMIT = 100
MAX_RECORDS_LIMIT = 1000
MAX_TRANSACTION_NOTE_CHARS = 20_000
MAX_TRANSACTION_TAG_CHARS = 128
MAX_PRICING_EXTERNAL_REF_CHARS = 500
SUPPORTED_PRICING_SOURCE_KINDS = set(pricing.SOURCE_PRIORITY)
SUPPORTED_PRICING_QUALITIES = {
    pricing.QUALITY_EXACT,
    pricing.QUALITY_PROVIDER_SAMPLE,
    pricing.QUALITY_COARSE_FALLBACK,
    pricing.QUALITY_MISSING,
}
SUPPORTED_REVIEW_STATUSES = {"completed", "pending", "failed", "review"}
SUPPORTED_AT_REGIME_OVERRIDES = {"alt", "neu", "outside"}
SUPPORTED_AT_CATEGORY_OVERRIDES = {
    "income_general",
    "income_capital_yield",
    "neu_gain",
    "neu_loss",
    "neu_swap",
    "alt_spekulation",
    "alt_taxfree",
    "none",
}
BIP329_PRESERVED_TYPES = {"pubkey", "xpub", "spscan"}
BIP329_EXPORT_MODES = {"stored", "synthesized", "all"}

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
WalletResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
TagResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
TransactionResolver = Callable[..., Mapping[str, Any]]
NormalizeCode = Callable[[Any], str]
NowIso = Callable[[], str]
InvalidateJournals = Callable[[sqlite3.Connection, str], None]
ParseIsoDateTime = Callable[[str, str], Any]
IsoFormatter = Callable[[Any], str]
EncodeCursor = Callable[[Mapping[str, Any], Mapping[str, Any]], str]
DecodeCursor = Callable[[str | None, Mapping[str, Any]], Mapping[str, str] | None]


@dataclass(frozen=True)
class MetadataHooks:
    resolve_scope: ScopeResolver
    resolve_wallet: WalletResolver
    resolve_tag: TagResolver
    resolve_transaction: TransactionResolver
    normalize_code: NormalizeCode
    now_iso: NowIso
    invalidate_journals: InvalidateJournals
    parse_iso_datetime: ParseIsoDateTime
    iso_z: IsoFormatter
    encode_cursor: EncodeCursor
    decode_cursor: DecodeCursor


def ensure_tag_row(conn, workspace_id, profile_id, code, label, hooks: MetadataHooks):
    normalized_code = hooks.normalize_code(code)
    existing = conn.execute(
        "SELECT * FROM tags WHERE profile_id = ? AND code = ?",
        (profile_id, normalized_code),
    ).fetchone()
    if existing:
        return existing, False
    tag_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
        VALUES(?, ?, ?, ?, ?, ?)
        """,
        (tag_id, workspace_id, profile_id, normalized_code, label, hooks.now_iso()),
    )
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone(), True


def set_transaction_note(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    note,
    hooks: MetadataHooks,
    *,
    source="cli",
    reason=None,
):
    record = update_transaction_metadata(
        conn,
        workspace_ref,
        profile_ref,
        tx_ref,
        hooks,
        note=note,
        note_set=True,
        source=source,
        reason=reason,
    )
    return {
        "transaction_id": record["transaction_id"],
        "note": note,
        "updated": record["updated"],
        "history_event_id": record["history_event_id"],
    }


def clear_transaction_note(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    hooks: MetadataHooks,
    *,
    source="cli",
    reason=None,
):
    return set_transaction_note(
        conn,
        workspace_ref,
        profile_ref,
        tx_ref,
        None,
        hooks,
        source=source,
        reason=reason,
    )


def set_transaction_excluded(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    excluded,
    hooks: MetadataHooks,
    *,
    source="cli",
    reason=None,
):
    record = update_transaction_metadata(
        conn,
        workspace_ref,
        profile_ref,
        tx_ref,
        hooks,
        excluded=bool(excluded),
        source=source,
        reason=reason,
    )
    return {
        "transaction_id": record["transaction_id"],
        "excluded": bool(excluded),
        "updated": record["updated"],
        "history_event_id": record["history_event_id"],
    }


def _guard_excluding_paired_leg(conn, profile_id, tx_id):
    """Refuse to exclude a transaction that is still a leg of an active pair or
    direct swap payout.

    The journal pipeline filters excluded rows but loads pair/payout records by
    ``deleted_at IS NULL`` only, so excluding one leg orphans the survivor into a
    phantom disposal (or a fresh-basis acquisition with the original basis lost).
    Require the user to reopen or supersede the authored review first.
    """
    from . import custody_authored_migration

    review = custody_authored_migration.find_active_review_for_transaction(
        conn,
        profile_id=profile_id,
        transaction_id=tx_id,
    )
    if review:
        review_id = review["id"]
        raise AppError(
            f"Transaction belongs to active custody review {review_id}; excluding "
            "it would orphan reviewed custody evidence.",
            code="conflict",
            hint=(
                "Reopen or supersede the custody review before excluding this "
                "transaction."
            ),
            details={
                "review_id": review_id,
                "component_id": review["component_id"],
                "term_kind": review["term_kind"],
            },
            retryable=False,
        )


def _clean_transaction_note(note):
    if note is None:
        return None
    if not isinstance(note, str):
        raise AppError("note must be a string or null", code="validation")
    if len(note) > MAX_TRANSACTION_NOTE_CHARS:
        raise AppError(
            f"note cannot exceed {MAX_TRANSACTION_NOTE_CHARS} characters",
            code="validation",
            retryable=False,
        )
    return note


def _clean_transaction_tags(tags):
    if tags is None:
        return None
    if not isinstance(tags, list):
        raise AppError("tags must be a list of strings", code="validation")
    cleaned = []
    seen = set()
    for tag in tags:
        if not isinstance(tag, str):
            raise AppError("tags must be a list of strings", code="validation")
        label = tag.strip()
        if not label:
            continue
        if len(label) > MAX_TRANSACTION_TAG_CHARS:
            raise AppError(
                f"tag cannot exceed {MAX_TRANSACTION_TAG_CHARS} characters",
                code="validation",
                details={"tag": label},
                retryable=False,
            )
        code = label.lower()
        if code in seen:
            continue
        cleaned.append(label)
        seen.add(code)
    return cleaned


def _clean_optional_string(value, field, *, max_chars):
    if value is None:
        return None
    if not isinstance(value, str):
        raise AppError(f"{field} must be a string or null", code="validation", retryable=False)
    cleaned = value.strip()
    if not cleaned:
        return None
    if len(cleaned) > max_chars:
        raise AppError(
            f"{field} cannot exceed {max_chars} characters",
            code="validation",
            retryable=False,
        )
    return cleaned


def _clean_fiat_currency(value, fallback):
    raw = value if value not in (None, "") else fallback
    if not isinstance(raw, str):
        raise AppError("fiat_currency must be a string", code="validation", retryable=False)
    cleaned = raw.strip().upper()
    if len(cleaned) != 3 or not cleaned.isalpha():
        raise AppError("fiat_currency must be a 3-letter currency code", code="validation", retryable=False)
    return cleaned


def _clean_decimal_or_none(value, field):
    if value in (None, ""):
        return None
    try:
        return dec(value)
    except Exception as exc:
        raise AppError(f"{field} must be a decimal number", code="validation", retryable=False) from exc


def _clean_optional_choice(value, field, choices):
    cleaned = _clean_optional_string(value, field, max_chars=64)
    if cleaned is None:
        return None
    if cleaned not in choices:
        raise AppError(
            f"{field} is not supported",
            code="validation",
            details={field: cleaned},
            retryable=False,
        )
    return cleaned


def _transaction_pricing_payload(
    profile,
    tx,
    *,
    fiat_currency=None,
    fiat_rate=None,
    fiat_value=None,
    source_kind=None,
    quality=None,
    external_ref=None,
    method=None,
):
    clean_source_kind = _clean_optional_string(source_kind, "pricing_source_kind", max_chars=64)
    clean_quality = _clean_optional_string(quality, "pricing_quality", max_chars=64)
    if clean_source_kind is not None and clean_source_kind not in SUPPORTED_PRICING_SOURCE_KINDS:
        raise AppError(
            "pricing_source_kind is not supported",
            code="validation",
            details={"pricing_source_kind": clean_source_kind},
            retryable=False,
        )
    if clean_quality is not None and clean_quality not in SUPPORTED_PRICING_QUALITIES:
        raise AppError(
            "pricing_quality is not supported",
            code="validation",
            details={"pricing_quality": clean_quality},
            retryable=False,
        )

    if clean_source_kind is None or clean_quality == pricing.QUALITY_MISSING:
        clean_currency = _clean_fiat_currency(fiat_currency, tx["fiat_currency"] or profile["fiat_currency"])
        return {
            "fiat_currency": clean_currency,
            **pricing.pricing_payload(
                rate=None,
                value=None,
                source_kind=None,
                quality=pricing.QUALITY_MISSING,
            ),
        }

    clean_quality = clean_quality or pricing.import_quality(clean_source_kind)
    clean_currency = _clean_fiat_currency(fiat_currency, tx["fiat_currency"] or profile["fiat_currency"])
    rate = _clean_decimal_or_none(fiat_rate, "fiat_rate")
    value = _clean_decimal_or_none(fiat_value, "fiat_value")
    amount = abs(msat_to_btc(tx["amount"]))
    if rate is None and value is not None and amount > 0:
        rate = value / amount
    if value is None and rate is not None and amount > 0:
        value = rate * amount
    if rate is None and value is None:
        raise AppError(
            "pricing updates require fiat_rate or fiat_value",
            code="validation",
            retryable=False,
        )
    if rate is not None and rate <= 0:
        raise AppError("fiat_rate must be positive", code="validation", retryable=False)
    if value is not None and value < 0:
        raise AppError("fiat_value must not be negative", code="validation", retryable=False)

    clean_external_ref = _clean_optional_string(
        external_ref,
        "pricing_external_ref",
        max_chars=MAX_PRICING_EXTERNAL_REF_CHARS,
    )
    clean_method = _clean_optional_string(method, "pricing_method", max_chars=64) or "desktop_transaction_detail"
    provider = "manual" if clean_source_kind == pricing.SOURCE_MANUAL_OVERRIDE else None
    return {
        "fiat_currency": clean_currency,
        **pricing.pricing_payload(
            rate=rate,
            value=value,
            source_kind=clean_source_kind,
            quality=clean_quality,
            provider=provider,
            pair=f"{tx['asset']}-{clean_currency}",
            pricing_timestamp=tx["confirmed_at"] or tx["occurred_at"],
            fetched_at=None,
            granularity="exact" if clean_quality == pricing.QUALITY_EXACT else None,
            method=clean_method,
            external_ref=clean_external_ref,
        ),
    }


def _current_profile_row(conn, profile_id):
    return conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,)).fetchone()


def _state_from_updates(before_state, state_updates, *, tags_set=False, tags=None):
    after_state = dict(before_state)
    if tags_set:
        after_state["tags"] = transaction_history.value_for_tx_update("tags", tags)
    for field, value in state_updates.items():
        after_state[field] = transaction_history.value_for_tx_update(field, value)
    return after_state


def _apply_audited_transaction_update(
    conn,
    *,
    workspace,
    profile,
    tx,
    hooks: MetadataHooks,
    tx_updates,
    state_updates,
    tags_set=False,
    tags=None,
    source="cli",
    reason=None,
    commit=True,
):
    source = transaction_history.normalize_source(source)
    reason = transaction_history.clean_reason(reason)
    before_tags = _tags_for_transaction(conn, tx["id"])
    before_state = transaction_history.transaction_state(tx, before_tags)
    after_state = _state_from_updates(
        before_state,
        state_updates,
        tags_set=tags_set,
        tags=tags,
    )
    requested_fields = sorted(
        set(state_updates) | ({"tags"} if tags_set else set()),
        key=lambda field: field,
    )
    changed_fields = [
        field
        for field in requested_fields
        if transaction_history.values_differ(before_state.get(field), after_state.get(field))
    ]
    if changed_fields and all(field == "pricing_fetched_at" for field in changed_fields):
        changed_fields = []
    if not changed_fields:
        return None, False

    try:
        if tags_set and "tags" in changed_fields:
            tag_rows = [
                ensure_tag_row(conn, workspace["id"], profile["id"], tag, tag, hooks)[0]
                for tag in (tags or [])
            ]
            conn.execute("DELETE FROM transaction_tags WHERE transaction_id = ?", (tx["id"],))
            conn.executemany(
                "INSERT OR IGNORE INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
                [(tx["id"], tag["id"]) for tag in tag_rows],
            )

        if tx_updates:
            assignments = [f"{column} = ?" for column in tx_updates]
            conn.execute(
                f"UPDATE transactions SET {', '.join(assignments)} WHERE id = ?",
                (*tx_updates.values(), tx["id"]),
            )

        fresh_profile = _current_profile_row(conn, profile["id"]) or profile
        event_id = transaction_history.append_event(
            conn,
            workspace=workspace,
            profile=fresh_profile,
            tx=tx,
            source=source,
            reason=reason,
            changed_at=hooks.now_iso(),
            changed_fields=changed_fields,
            before_state=before_state,
            after_state=after_state,
        )
        hooks.invalidate_journals(conn, profile["id"])
        if commit:
            conn.commit()
        return event_id, True
    except Exception:
        conn.rollback()
        raise


def update_transaction_metadata(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    hooks: MetadataHooks,
    *,
    note=None,
    note_set=False,
    tags=None,
    excluded=None,
    pricing_update=None,
    review_status=None,
    review_status_set=False,
    taxable=None,
    taxable_set=False,
    at_regime=None,
    at_regime_set=False,
    at_category=None,
    at_category_set=False,
    source="cli",
    reason=None,
    commit=True,
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    clean_note = _clean_transaction_note(note) if note_set else None
    clean_tags = _clean_transaction_tags(tags)
    if excluded is not None and not isinstance(excluded, bool):
        raise AppError("excluded must be a boolean", code="validation", retryable=False)
    if excluded is True:
        _guard_excluding_paired_leg(conn, profile["id"], tx["id"])
    if taxable_set and taxable is not None and not isinstance(taxable, bool):
        raise AppError("taxable must be a boolean", code="validation", retryable=False)
    clean_review_status = (
        _clean_optional_choice(review_status, "review_status", SUPPORTED_REVIEW_STATUSES)
        if review_status_set and review_status is not None
        else None
    )
    clean_at_regime = (
        _clean_optional_choice(at_regime, "at_regime", SUPPORTED_AT_REGIME_OVERRIDES)
        if at_regime_set and at_regime is not None
        else None
    )
    clean_at_category = (
        _clean_optional_choice(at_category, "at_category", SUPPORTED_AT_CATEGORY_OVERRIDES)
        if at_category_set and at_category is not None
        else None
    )
    clean_pricing = None
    if pricing_update is not None:
        if not isinstance(pricing_update, Mapping):
            raise AppError("pricing must be an object", code="validation", retryable=False)
        clean_pricing = _transaction_pricing_payload(profile, tx, **pricing_update)
        if clean_pricing["pricing_source_kind"] is not None:
            clean_pricing["pricing_fetched_at"] = hooks.now_iso()

    tx_updates = {}
    state_updates = {}
    if note_set:
        tx_updates["note"] = clean_note
        state_updates["note"] = clean_note

    if excluded is not None:
        tx_updates["excluded"] = 1 if excluded else 0
        state_updates["excluded"] = excluded

    if review_status_set:
        tx_updates["review_status"] = clean_review_status
        state_updates["review_status"] = clean_review_status

    if taxable_set:
        tx_updates["taxability_override"] = None if taxable is None else (1 if taxable else 0)
        state_updates["taxable"] = taxable

    if at_regime_set:
        tx_updates["at_regime_override"] = clean_at_regime
        state_updates["at_regime"] = clean_at_regime

    if at_category_set:
        tx_updates["at_category_override"] = clean_at_category
        state_updates["at_category"] = clean_at_category

    if clean_pricing is not None:
        tx_updates.update(
            {
                "fiat_currency": clean_pricing["fiat_currency"],
                "fiat_rate": clean_pricing["fiat_rate"],
                "fiat_value": clean_pricing["fiat_value"],
                "fiat_price_source": clean_pricing["fiat_price_source"],
                "fiat_rate_exact": clean_pricing["fiat_rate_exact"],
                "fiat_value_exact": clean_pricing["fiat_value_exact"],
                "pricing_source_kind": clean_pricing["pricing_source_kind"],
                "pricing_provider": clean_pricing["pricing_provider"],
                "pricing_pair": clean_pricing["pricing_pair"],
                "pricing_timestamp": clean_pricing["pricing_timestamp"],
                "pricing_fetched_at": clean_pricing["pricing_fetched_at"],
                "pricing_granularity": clean_pricing["pricing_granularity"],
                "pricing_method": clean_pricing["pricing_method"],
                "pricing_external_ref": clean_pricing["pricing_external_ref"],
                "pricing_quality": clean_pricing["pricing_quality"],
            }
        )
        state_updates.update(
            {
                "fiat_currency": clean_pricing["fiat_currency"],
                "fiat_rate": clean_pricing["fiat_rate_exact"],
                "fiat_value": clean_pricing["fiat_value_exact"],
                "fiat_price_source": clean_pricing["fiat_price_source"],
                "pricing_source_kind": clean_pricing["pricing_source_kind"],
                "pricing_provider": clean_pricing["pricing_provider"],
                "pricing_pair": clean_pricing["pricing_pair"],
                "pricing_timestamp": clean_pricing["pricing_timestamp"],
                "pricing_fetched_at": clean_pricing["pricing_fetched_at"],
                "pricing_granularity": clean_pricing["pricing_granularity"],
                "pricing_method": clean_pricing["pricing_method"],
                "pricing_external_ref": clean_pricing["pricing_external_ref"],
                "pricing_quality": clean_pricing["pricing_quality"],
            }
        )

    event_id, changed = _apply_audited_transaction_update(
        conn,
        workspace=workspace,
        profile=profile,
        tx=tx,
        hooks=hooks,
        tx_updates=tx_updates,
        state_updates=state_updates,
        tags_set=clean_tags is not None,
        tags=clean_tags,
        source=source,
        reason=reason,
        commit=commit,
    )

    record = get_transaction_record(conn, workspace_ref, profile_ref, tx["id"], hooks)
    record["updated"] = changed
    record["history_event_id"] = event_id
    return record


def create_tag(conn, workspace_ref, profile_ref, code, label, hooks: MetadataHooks):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tag_id = str(uuid.uuid4())
    normalized_code = hooks.normalize_code(code)
    try:
        conn.execute(
            """
            INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at)
            VALUES(?, ?, ?, ?, ?, ?)
            """,
            (tag_id, workspace["id"], profile["id"], normalized_code, label, hooks.now_iso()),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"Tag '{normalized_code}' already exists",
            code="conflict",
            hint="Choose a different tag code or use the existing tag.",
        ) from exc
    conn.commit()
    return conn.execute("SELECT * FROM tags WHERE id = ?", (tag_id,)).fetchone()


def list_tags(conn, workspace_ref, profile_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        "SELECT id, code, label, created_at FROM tags WHERE profile_id = ? ORDER BY code ASC",
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


def add_tag_to_transaction(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    tag_ref,
    hooks: MetadataHooks,
    *,
    source="cli",
    reason=None,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    tag = hooks.resolve_tag(conn, profile["id"], tag_ref)
    current_tags = [row["label"] for row in _tags_for_transaction(conn, tx["id"])]
    record = update_transaction_metadata(
        conn,
        workspace_ref,
        profile_ref,
        tx["id"],
        hooks,
        tags=[*current_tags, tag["label"]],
        source=source,
        reason=reason,
    )
    return {
        "transaction_id": tx["id"],
        "tag": tag["code"],
        "status": "added" if record["updated"] else "unchanged",
        "updated": record["updated"],
        "history_event_id": record["history_event_id"],
    }


def remove_tag_from_transaction(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    tag_ref,
    hooks: MetadataHooks,
    *,
    source="cli",
    reason=None,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    tag = hooks.resolve_tag(conn, profile["id"], tag_ref)
    current_tags = [row["label"] for row in _tags_for_transaction(conn, tx["id"])]
    next_tags = [label for label in current_tags if label.lower() != tag["label"].lower()]
    record = update_transaction_metadata(
        conn,
        workspace_ref,
        profile_ref,
        tx["id"],
        hooks,
        tags=next_tags,
        source=source,
        reason=reason,
    )
    return {
        "transaction_id": tx["id"],
        "tag": tag["code"],
        "status": "removed" if record["updated"] else "unchanged",
        "updated": record["updated"],
        "history_event_id": record["history_event_id"],
    }


def _tags_for_transaction(conn, tx_id):
    rows = conn.execute(
        """
        SELECT t.code, t.label
        FROM transaction_tags tt
        JOIN tags t ON t.id = tt.tag_id
        WHERE tt.transaction_id = ?
        ORDER BY t.code ASC
        """,
        (tx_id,),
    ).fetchall()
    return [{"code": row["code"], "label": row["label"]} for row in rows]


def get_transaction_record(conn, workspace_ref, profile_ref, tx_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    wallet = conn.execute(
        "SELECT id, label FROM wallets WHERE id = ?",
        (tx["wallet_id"],),
    ).fetchone()
    return {
        "transaction_id": tx["id"],
        "external_id": tx["external_id"] or "",
        "occurred_at": tx["occurred_at"],
        "direction": tx["direction"],
        "asset": tx["asset"],
        "amount": float(msat_to_btc(tx["amount"])),
        "amount_msat": int(tx["amount"]),
        "fee": float(msat_to_btc(tx["fee"])),
        "fee_msat": int(tx["fee"]),
        "counterparty": tx["counterparty"] or "",
        "wallet_id": wallet["id"] if wallet else "",
        "wallet_label": wallet["label"] if wallet else "",
        "note": tx["note"] or "",
        "excluded": bool(tx["excluded"]),
        "fiat_currency": tx["fiat_currency"],
        "fiat_rate": tx["fiat_rate"],
        "fiat_value": tx["fiat_value"],
        "fiat_rate_exact": tx["fiat_rate_exact"],
        "fiat_value_exact": tx["fiat_value_exact"],
        "pricing_source_kind": tx["pricing_source_kind"],
        "pricing_quality": tx["pricing_quality"],
        "pricing_external_ref": tx["pricing_external_ref"],
        "review_status": tx["review_status"],
        "taxable": None if tx["taxability_override"] is None else bool(tx["taxability_override"]),
        "at_regime": tx["at_regime_override"],
        "at_category": tx["at_category_override"],
        "tags": _tags_for_transaction(conn, tx["id"]),
    }


def list_transaction_records(
    conn,
    workspace_ref,
    profile_ref,
    hooks: MetadataHooks,
    wallet=None,
    tag=None,
    has_note=None,
    excluded=None,
    start=None,
    end=None,
    cursor=None,
    limit=None,
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_RECORDS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_RECORDS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_RECORDS_LIMIT}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_RECORDS_LIMIT}.",
        )

    where = ["t.profile_id = ?"]
    params = [profile["id"]]
    start_ts = hooks.iso_z(hooks.parse_iso_datetime(start, "start")) if start else None
    end_ts = hooks.iso_z(hooks.parse_iso_datetime(end, "end")) if end else None

    wallet_id = ""
    tag_id = ""
    if wallet:
        wallet_row = hooks.resolve_wallet(conn, profile["id"], wallet)
        wallet_id = wallet_row["id"]
        where.append("t.wallet_id = ?")
        params.append(wallet_id)
    if tag:
        tag_row = hooks.resolve_tag(conn, profile["id"], tag)
        tag_id = tag_row["id"]
        where.append("EXISTS (SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id = t.id AND tt.tag_id = ?)")
        params.append(tag_id)
    if has_note is True:
        where.append("t.note IS NOT NULL AND t.note != ''")
    elif has_note is False:
        where.append("(t.note IS NULL OR t.note = '')")
    if excluded is True:
        where.append("t.excluded = 1")
    elif excluded is False:
        where.append("t.excluded = 0")
    if start_ts:
        where.append("t.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("t.occurred_at <= ?")
        params.append(end_ts)

    cursor_filters = {
        "workspace_id": workspace["id"],
        "profile_id": profile["id"],
        "wallet_id": wallet_id,
        "tag_id": tag_id,
        "has_note": has_note,
        "excluded": excluded,
        "start": start_ts or "",
        "end": end_ts or "",
    }
    cursor_data = hooks.decode_cursor(cursor, cursor_filters)
    if cursor_data:
        where.append(
            "(t.occurred_at < ? OR "
            "(t.occurred_at = ? AND t.created_at < ?) OR "
            "(t.occurred_at = ? AND t.created_at = ? AND t.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )

    query = f"""
        SELECT
            t.id,
            t.occurred_at,
            t.created_at,
            t.external_id,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.counterparty,
            t.note,
            t.excluded,
            w.id AS wallet_id,
            w.label AS wallet_label
        FROM transactions t
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE {' AND '.join(where)}
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT ?
    """
    params.append(effective_limit + 1)
    rows = conn.execute(query, params).fetchall()

    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    records = []
    for row in page:
        records.append(
            {
                "transaction_id": row["id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"])),
                "amount_msat": int(row["amount"]),
                "fee": float(msat_to_btc(row["fee"])),
                "fee_msat": int(row["fee"]),
                "counterparty": row["counterparty"] or "",
                "wallet_id": row["wallet_id"] or "",
                "wallet_label": row["wallet_label"] or "",
                "note": row["note"] or "",
                "excluded": bool(row["excluded"]),
                "tags": _tags_for_transaction(conn, row["id"]),
            }
        )
    next_cursor = hooks.encode_cursor(page[-1], cursor_filters) if has_more and page else None
    return {
        "records": records,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def list_transaction_history(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    hooks: MetadataHooks,
    *,
    source=None,
    field_family=None,
    field=None,
    pricing_only=False,
    ai_only=False,
    stale_only=False,
    start=None,
    end=None,
    cursor=None,
    limit=None,
    include_stale=True,
):
    return transaction_history.list_history(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        transaction_ref=tx_ref,
        source=source,
        field_family=field_family,
        field=field,
        pricing_only=pricing_only,
        ai_only=ai_only,
        stale_only=stale_only,
        start=start,
        end=end,
        cursor=cursor,
        limit=limit,
        include_stale=include_stale,
    )


def list_activity_history(
    conn,
    workspace_ref,
    profile_ref,
    hooks: MetadataHooks,
    *,
    transaction_ref=None,
    wallet_ref=None,
    source=None,
    field_family=None,
    field=None,
    pricing_only=False,
    ai_only=False,
    stale_only=False,
    start=None,
    end=None,
    cursor=None,
    limit=None,
    include_stale=True,
):
    return transaction_history.list_history(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        transaction_ref=transaction_ref,
        wallet_ref=wallet_ref,
        source=source,
        field_family=field_family,
        field=field,
        pricing_only=pricing_only,
        ai_only=ai_only,
        stale_only=stale_only,
        start=start,
        end=end,
        cursor=cursor,
        limit=limit,
        include_stale=include_stale,
    )


def stale_transaction_edit_summary(conn, workspace_ref, profile_ref, hooks: MetadataHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    return transaction_history.stale_summary(conn, profile)


def _tx_updates_for_revert_field(field, before_value):
    if field == "note":
        return {"note": before_value}, {"note": before_value}
    if field == "excluded":
        return {"excluded": 1 if before_value else 0}, {"excluded": bool(before_value)}
    if field == "review_status":
        return {"review_status": before_value}, {"review_status": before_value}
    if field == "taxable":
        return {
            "taxability_override": None if before_value is None else (1 if before_value else 0)
        }, {"taxable": before_value}
    if field == "at_regime":
        return {"at_regime_override": before_value}, {"at_regime": before_value}
    if field == "at_category":
        return {"at_category_override": before_value}, {"at_category": before_value}
    if field == "fiat_rate":
        return {
            "fiat_rate": None if before_value is None else float(dec(before_value)),
            "fiat_rate_exact": before_value,
        }, {"fiat_rate": before_value}
    if field == "fiat_value":
        return {
            "fiat_value": None if before_value is None else float(dec(before_value)),
            "fiat_value_exact": before_value,
        }, {"fiat_value": before_value}
    column_fields = {
        "fiat_currency",
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
    if field in column_fields:
        return {field: before_value}, {field: before_value}
    raise AppError(
        "history field cannot be reverted",
        code="validation",
        details={"field": field},
        retryable=False,
    )


def revert_transaction_edit(
    conn,
    workspace_ref,
    profile_ref,
    tx_ref,
    hooks: MetadataHooks,
    *,
    event_id,
    field=None,
    source="cli",
    reason=None,
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    loaded = transaction_history.load_event_for_revert(
        conn,
        profile_id=profile["id"],
        transaction_id=tx["id"],
        event_id=event_id,
    )
    fields = loaded["fields"]
    if field:
        if field not in transaction_history.SUPPORTED_FIELDS:
            raise AppError(
                "history field is not supported",
                code="validation",
                details={"field": field},
                retryable=False,
            )
        fields = [row for row in fields if row["field"] == field]
        if not fields:
            raise AppError(
                "history event did not change that field",
                code="validation",
                details={"event_id": event_id, "field": field},
                retryable=False,
            )
    tx_updates = {}
    state_updates = {}
    tags_set = False
    tags = None
    reverted_fields = []
    for item in fields:
        edit_field = item["field"]
        before_value = item["before_value"]
        if edit_field == "tags":
            tags_set = True
            tags = before_value or []
            reverted_fields.append(edit_field)
            continue
        next_tx_updates, next_state_updates = _tx_updates_for_revert_field(edit_field, before_value)
        tx_updates.update(next_tx_updates)
        state_updates.update(next_state_updates)
        reverted_fields.append(edit_field)
    event_reason = reason or f"Reverted transaction edit {event_id}"
    new_event_id, changed = _apply_audited_transaction_update(
        conn,
        workspace=workspace,
        profile=profile,
        tx=tx,
        hooks=hooks,
        tx_updates=tx_updates,
        state_updates=state_updates,
        tags_set=tags_set,
        tags=tags,
        source=source,
        reason=event_reason,
    )
    record = get_transaction_record(conn, workspace_ref, profile_ref, tx["id"], hooks)
    return {
        "transaction": record,
        "updated": changed,
        "reverted_event_id": event_id,
        "history_event_id": new_event_id,
        "reverted_fields": sorted(reverted_fields),
    }


def _decode_bip329_data(raw_json):
    try:
        payload = json.loads(raw_json or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return payload if isinstance(payload, dict) else {}


def _parse_bip329_outpoint(ref: Any) -> tuple[str, int] | None:
    text = str(ref or "").strip()
    if ":" not in text:
        return None
    txid, vout_text = text.rsplit(":", 1)
    txid = txid.strip()
    if not txid:
        return None
    try:
        vout = int(vout_text)
    except (TypeError, ValueError):
        return None
    if vout < 0:
        return None
    return txid, vout


def _redact_bip329_ref(record_type: str, ref: str) -> tuple[str, bool]:
    if record_type not in {"pubkey", "xpub", "spscan"}:
        return ref, False
    text = str(ref or "")
    if len(text) <= 16:
        return "[redacted]", True
    return f"{text[:8]}...{text[-8:]}", True


def _wallet_match_from_owned_matches(matches: Sequence[Any], *, source: str) -> dict[str, Any]:
    unique: dict[str, str] = {}
    match_sources: set[str] = set()
    for match in matches:
        wallet_id = str(match.wallet_id)
        unique.setdefault(wallet_id, str(match.wallet_label))
        if getattr(match, "source", None):
            match_sources.add(str(match.source))
    wallets = [
        {"wallet_id": wallet_id, "wallet": label}
        for wallet_id, label in sorted(unique.items(), key=lambda item: (item[1], item[0]))
    ]
    if not wallets:
        return {
            "status": "unmatched",
            "confidence": "none",
            "wallet_ids": [],
            "wallets": [],
            "match_source": source,
        }
    return {
        "status": "exact" if len(wallets) == 1 else "ambiguous",
        "confidence": "deterministic",
        "wallet_ids": [wallet["wallet_id"] for wallet in wallets],
        "wallets": [wallet["wallet"] for wallet in wallets],
        "match_source": ",".join(sorted(match_sources)) if match_sources else source,
    }


def _wallet_match_from_transaction_rows(rows: Sequence[sqlite3.Row]) -> dict[str, Any]:
    unique: dict[str, str] = {}
    for row in rows:
        wallet_id = str(row["wallet_id"] or "")
        if not wallet_id:
            continue
        unique.setdefault(wallet_id, str(row["wallet_label"] or wallet_id))
    wallets = [
        {"wallet_id": wallet_id, "wallet": label}
        for wallet_id, label in sorted(unique.items(), key=lambda item: (item[1], item[0]))
    ]
    if not rows:
        return {
            "status": "unmatched",
            "confidence": "none",
            "wallet_ids": [],
            "wallets": [],
            "match_source": "transactions.external_id",
        }
    if len(rows) == 1 and len(wallets) == 1:
        status = "exact"
    else:
        status = "ambiguous"
    return {
        "status": status,
        "confidence": "deterministic",
        "wallet_ids": [wallet["wallet_id"] for wallet in wallets],
        "wallets": [wallet["wallet"] for wallet in wallets],
        "match_source": "transactions.external_id",
    }


def _preserved_bip329_match(record_type: str) -> dict[str, Any]:
    return {
        "status": "preserved",
        "confidence": "none",
        "wallet_ids": [],
        "wallets": [],
        "match_source": record_type,
    }


def _match_bip329_record(
    conn: sqlite3.Connection,
    profile_id: str,
    record: Mapping[str, Any],
    index: ownership.OwnedIndex,
) -> tuple[dict[str, Any], list[sqlite3.Row]]:
    record_type = str(record["type"])
    ref = str(record["ref"])
    if record_type == "tx":
        rows = conn.execute(
            """
            SELECT t.id, t.wallet_id, w.label AS wallet_label
            FROM transactions t
            LEFT JOIN wallets w ON w.id = t.wallet_id
            WHERE t.profile_id = ? AND lower(t.external_id) = lower(?)
            ORDER BY w.label ASC, t.occurred_at ASC, t.created_at ASC, t.id ASC
            """,
            (profile_id, ref),
        ).fetchall()
        return _wallet_match_from_transaction_rows(rows), rows
    if record_type == "addr":
        verdict = ownership.classify_address({"input": ref}, index)
        matches = verdict.get("matches") or []
        if not matches:
            return {
                "status": "unmatched",
                "confidence": "none",
                "wallet_ids": [],
                "wallets": [],
                "match_source": "address",
            }, []
        wallets = {
            str(match.get("wallet_id") or ""): str(match.get("wallet") or match.get("wallet_id") or "")
            for match in matches
            if match.get("wallet_id")
        }
        ordered = [
            {"wallet_id": wallet_id, "wallet": label}
            for wallet_id, label in sorted(wallets.items(), key=lambda item: (item[1], item[0]))
        ]
        return {
            "status": "ambiguous" if verdict.get("ownership_ambiguous") else "exact",
            "confidence": "deterministic",
            "wallet_ids": [wallet["wallet_id"] for wallet in ordered],
            "wallets": [wallet["wallet"] for wallet in ordered],
            "match_source": "address",
        }, []
    if record_type == "output":
        outpoint = _parse_bip329_outpoint(ref)
        matches = index.lookup_outpoint(f"{outpoint[0]}:{outpoint[1]}") if outpoint else []
        return _wallet_match_from_owned_matches(matches, source="wallet_utxos.outpoint"), []
    if record_type == "input":
        outpoint = _parse_bip329_outpoint(ref)
        if outpoint is None:
            return {
                "status": "unmatched",
                "confidence": "none",
                "wallet_ids": [],
                "wallets": [],
                "match_source": "input_ref",
            }, []
        txid, input_index = outpoint
        legs = ownership.load_local_tx_legs(conn, profile_id, txid)
        matches = []
        if legs is not None:
            inputs = legs.get("inputs") or []
            if 0 <= input_index < len(inputs):
                leg = inputs[input_index]
                matches = index.lookup_outpoint(leg.get("outpoint"))
                if not matches:
                    matches = index.lookup_script(leg.get("script"))
        return _wallet_match_from_owned_matches(matches, source="local_tx.input"), []
    if record_type in BIP329_PRESERVED_TYPES:
        return _preserved_bip329_match(record_type), []
    return {
        "status": "unmatched",
        "confidence": "none",
        "wallet_ids": [],
        "wallets": [],
        "match_source": "unsupported",
    }, []


def _existing_bip329_row(
    conn: sqlite3.Connection,
    profile_id: str,
    record: Mapping[str, Any],
) -> sqlite3.Row | None:
    return conn.execute(
        """
        SELECT *
        FROM bip329_labels
        WHERE profile_id = ?
          AND record_type = ?
          AND ref = ?
        LIMIT 1
        """,
        (
            profile_id,
            record["type"],
            record["ref"],
        ),
    ).fetchone()


def _bip329_record_conflicts(existing: sqlite3.Row | None, record: Mapping[str, Any]) -> list[str]:
    if not existing:
        return []
    conflicts: list[str] = []
    for field in ("label", "origin"):
        incoming = record.get(field)
        if incoming is not None and existing[field] is not None and incoming != existing[field]:
            conflicts.append(field)
    incoming_spendable = record.get("spendable")
    if incoming_spendable is not None and existing["spendable"] is not None:
        if bool(existing["spendable"]) != bool(incoming_spendable):
            conflicts.append("spendable")
    existing_data = _decode_bip329_data(existing["data_json"])
    for key, value in (record.get("data") or {}).items():
        if key in existing_data and existing_data[key] != value:
            conflicts.append(f"data.{key}")
    return conflicts


def _merge_bip329_import_match_data(
    record_data: Mapping[str, Any],
    match_info: Mapping[str, Any],
) -> dict[str, Any]:
    data = dict(record_data or {})
    imported_kassiber = data.get("kassiber")
    kassiber_data = dict(imported_kassiber) if isinstance(imported_kassiber, dict) else {}
    if imported_kassiber is not None and not isinstance(imported_kassiber, dict):
        kassiber_data.setdefault("imported_value", imported_kassiber)
    kassiber_data["wallet_match"] = {
        "status": match_info.get("status"),
        "confidence": match_info.get("confidence"),
        "wallet_ids": list(match_info.get("wallet_ids") or []),
        "wallets": list(match_info.get("wallets") or []),
        "match_source": match_info.get("match_source"),
    }
    data["kassiber"] = kassiber_data
    return data


def _planned_tx_tag_effects(
    conn: sqlite3.Connection,
    profile_id: str,
    label: str | None,
    tx_rows: Sequence[sqlite3.Row],
    *,
    match_status: str,
    project_tags: bool,
    apply_ambiguous: bool,
    hooks: MetadataHooks,
) -> list[dict[str, Any]]:
    if not label:
        return []
    if len(label.strip()) > MAX_TRANSACTION_TAG_CHARS:
        return [
            {
                "action": "skipped_label_too_long",
                "reason": f"BIP329 labels projected to tags cannot exceed {MAX_TRANSACTION_TAG_CHARS} characters.",
            }
        ]
    if not project_tags:
        return [{"action": "skipped_duplicate", "reason": "A later duplicate record wins."}]
    if match_status == "ambiguous" and not apply_ambiguous:
        return [
            {
                "transaction_id": row["id"],
                "wallet": row["wallet_label"] or "",
                "wallet_id": row["wallet_id"] or "",
                "action": "skipped_ambiguous",
            }
            for row in tx_rows
        ]
    if match_status != "exact" and not (match_status == "ambiguous" and apply_ambiguous):
        return []
    normalized = hooks.normalize_code(label)
    existing_tag = conn.execute(
        "SELECT label FROM tags WHERE profile_id = ? AND code = ? LIMIT 1",
        (profile_id, normalized),
    ).fetchone()
    effects: list[dict[str, Any]] = []
    for row in tx_rows:
        current_tags = _tags_for_transaction(conn, row["id"])
        has_tag = any(tag["code"] == normalized for tag in current_tags)
        action = "unchanged" if has_tag else "add"
        effect = {
            "transaction_id": row["id"],
            "wallet": row["wallet_label"] or "",
            "wallet_id": row["wallet_id"] or "",
            "tag": normalized,
            "label": label,
            "action": action,
        }
        if existing_tag and existing_tag["label"] != label:
            effect["conflict"] = "tag_label"
            effect["existing_label"] = existing_tag["label"]
        effects.append(effect)
    return effects


def _bip329_plan_counts(rows: Sequence[Mapping[str, Any]], duplicate_refs: int) -> dict[str, int]:
    counts = {
        "exact": 0,
        "ambiguous": 0,
        "unmatched": 0,
        "preserved": 0,
        "conflicts": 0,
        "duplicate_refs": duplicate_refs,
        "duplicate_records": 0,
        "tag_additions": 0,
        "tag_unchanged": 0,
        "tag_skipped_ambiguous": 0,
        "tag_skipped_duplicate": 0,
        "tag_skipped_label_too_long": 0,
    }
    for row in rows:
        status = str(row.get("match_status") or "unmatched")
        if status in counts:
            counts[status] += 1
        if row.get("duplicate"):
            counts["duplicate_records"] += 1
        if row.get("conflicts"):
            counts["conflicts"] += 1
        for effect in row.get("tag_effects") or []:
            action = effect.get("action")
            if action == "add":
                counts["tag_additions"] += 1
            elif action == "unchanged":
                counts["tag_unchanged"] += 1
            elif action == "skipped_ambiguous":
                counts["tag_skipped_ambiguous"] += 1
            elif action == "skipped_duplicate":
                counts["tag_skipped_duplicate"] += 1
            elif action == "skipped_label_too_long":
                counts["tag_skipped_label_too_long"] += 1
    return counts


def _plan_bip329_import(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    file_path: str,
    hooks: MetadataHooks,
    *,
    apply_ambiguous: bool = False,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    records = load_bip329_file(file_path)
    wallets = ownership.load_profile_wallets(conn, profile["id"])
    owned_index, warnings = ownership.build_owned_index(conn, profile["id"], wallets)
    key_counts: dict[tuple[str, str], int] = {}
    last_index_by_key: dict[tuple[str, str], int] = {}
    for index, record in enumerate(records):
        key = (record["type"], record["ref"])
        key_counts[key] = key_counts.get(key, 0) + 1
        last_index_by_key[key] = index
    duplicate_refs = sum(1 for count in key_counts.values() if count > 1)
    rows: list[dict[str, Any]] = []
    seen_by_key: dict[tuple[str, str], int] = {}
    for index, record in enumerate(records):
        key = (record["type"], record["ref"])
        seen_by_key[key] = seen_by_key.get(key, 0) + 1
        match_info, tx_rows = _match_bip329_record(conn, profile["id"], record, owned_index)
        existing = _existing_bip329_row(conn, profile["id"], record)
        conflicts = _bip329_record_conflicts(existing, record)
        ref_preview, ref_redacted = _redact_bip329_ref(record["type"], record["ref"])
        is_duplicate = key_counts[key] > 1
        project_tags = record["type"] == "tx" and index == last_index_by_key[key]
        tag_effects = (
            _planned_tx_tag_effects(
                conn,
                profile["id"],
                record["label"],
                tx_rows,
                match_status=match_info["status"],
                project_tags=project_tags,
                apply_ambiguous=apply_ambiguous,
                hooks=hooks,
            )
            if record["type"] == "tx"
            else []
        )
        rows.append(
            {
                "_record": record,
                "_tx_rows": tx_rows,
                "line": index + 1,
                "type": record["type"],
                "ref": ref_preview if ref_redacted else record["ref"],
                "ref_preview": ref_preview,
                "ref_redacted": ref_redacted,
                "label": record["label"] or "",
                "origin": record["origin"] or "",
                "match_status": match_info["status"],
                "match_confidence": match_info["confidence"],
                "wallet_ids": list(match_info.get("wallet_ids") or []),
                "wallets": list(match_info.get("wallets") or []),
                "match_source": match_info.get("match_source") or "",
                "duplicate": is_duplicate,
                "duplicate_ordinal": seen_by_key[key] if is_duplicate else None,
                "conflicts": conflicts,
                "existing": {
                    "label": existing["label"] if existing else None,
                    "origin": existing["origin"] if existing else None,
                    "spendable": (None if not existing or existing["spendable"] is None else bool(existing["spendable"])),
                }
                if existing
                else None,
                "tag_effects": tag_effects,
                "wallet_match": match_info,
            }
        )
    return {
        "file": os.path.abspath(file_path),
        "workspace_id": workspace["id"],
        "profile_id": profile["id"],
        "records": len(records),
        "rows": rows,
        "counts": _bip329_plan_counts(rows, duplicate_refs),
        "warnings": warnings,
        "apply_ambiguous": bool(apply_ambiguous),
    }


def _public_bip329_plan(plan: Mapping[str, Any], *, include_rows: bool = True) -> dict[str, Any]:
    public = {
        "file": plan["file"],
        "records": plan["records"],
        "counts": plan["counts"],
        "warnings": plan.get("warnings") or [],
        "apply_policy": "all_matches" if plan.get("apply_ambiguous") else "exact_only",
    }
    if include_rows:
        rows = []
        for row in plan.get("rows") or []:
            rows.append(
                {
                    key: value
                    for key, value in row.items()
                    if not key.startswith("_") and key != "wallet_match"
                }
            )
        public["rows"] = rows
    return public


def preview_bip329_import(conn, workspace_ref, profile_ref, file_path, hooks: MetadataHooks):
    return _public_bip329_plan(
        _plan_bip329_import(conn, workspace_ref, profile_ref, file_path, hooks),
        include_rows=True,
    )


def _upsert_bip329_record(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    record: Mapping[str, Any],
    match_info: Mapping[str, Any],
    hooks: MetadataHooks,
) -> str:
    existing = _existing_bip329_row(conn, profile_id, record)
    record_data = _merge_bip329_import_match_data(record.get("data") or {}, match_info)
    if existing:
        effective_label = record["label"] if record["label"] is not None else existing["label"]
        effective_origin = record["origin"] if record["origin"] is not None else existing["origin"]
        effective_spendable = (
            existing["spendable"]
            if record["spendable"] is None
            else (1 if record["spendable"] else 0)
        )
        merged_data = _decode_bip329_data(existing["data_json"])
        imported_kassiber = merged_data.get("kassiber")
        merged_data.update(record_data)
        if isinstance(imported_kassiber, dict) and isinstance(record_data.get("kassiber"), dict):
            kassiber_data = dict(imported_kassiber)
            kassiber_data.update(record_data["kassiber"])
            merged_data["kassiber"] = kassiber_data
        conn.execute(
            """
            UPDATE bip329_labels
            SET wallet_id = NULL,
                label = ?,
                origin = ?,
                spendable = ?,
                data_json = ?
            WHERE id = ?
            """,
            (
                effective_label,
                effective_origin,
                effective_spendable,
                json.dumps(merged_data, sort_keys=True),
                existing["id"],
            ),
        )
        return "updated"
    conn.execute(
        """
        INSERT INTO bip329_labels(
            id, workspace_id, profile_id, wallet_id, record_type, ref,
            label, origin, spendable, data_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            str(uuid.uuid4()),
            workspace_id,
            profile_id,
            None,
            record["type"],
            record["ref"],
            record["label"],
            record["origin"],
            None if record["spendable"] is None else (1 if record["spendable"] else 0),
            json.dumps(record_data, sort_keys=True),
            hooks.now_iso(),
        ),
    )
    return "imported"


def _apply_bip329_tag_effect(
    conn: sqlite3.Connection,
    workspace: Mapping[str, Any],
    profile: Mapping[str, Any],
    effect: Mapping[str, Any],
    hooks: MetadataHooks,
    *,
    source: str,
    reason: str,
) -> tuple[bool, bool]:
    if effect.get("action") != "add":
        return False, False
    label = str(effect.get("label") or "").strip()
    if not label:
        return False, False
    tx = conn.execute(
        "SELECT * FROM transactions WHERE profile_id = ? AND id = ? LIMIT 1",
        (profile["id"], effect.get("transaction_id")),
    ).fetchone()
    if not tx:
        return False, False
    _tag, tag_created = ensure_tag_row(conn, workspace["id"], profile["id"], label, label, hooks)
    current_tags = _tags_for_transaction(conn, tx["id"])
    normalized = hooks.normalize_code(label)
    if any(tag["code"] == normalized for tag in current_tags):
        return tag_created, False
    next_tags = [tag["label"] for tag in current_tags] + [label]
    _event_id, changed = _apply_audited_transaction_update(
        conn,
        workspace=workspace,
        profile=profile,
        tx=tx,
        hooks=hooks,
        tx_updates={},
        state_updates={},
        tags_set=True,
        tags=next_tags,
        source=source,
        reason=reason,
        commit=False,
    )
    return tag_created, changed


def import_bip329_labels(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: MetadataHooks,
    *,
    apply_ambiguous: bool = False,
    source: str = "cli",
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    plan = _plan_bip329_import(
        conn,
        workspace_ref,
        profile_ref,
        file_path,
        hooks,
        apply_ambiguous=apply_ambiguous,
    )
    imported = 0
    updated = 0
    transaction_tags_added = 0
    transaction_tags_created = 0
    try:
        for row in plan["rows"]:
            status = _upsert_bip329_record(
                conn,
                workspace["id"],
                profile["id"],
                row["_record"],
                row["wallet_match"],
                hooks,
            )
            if status == "imported":
                imported += 1
            else:
                updated += 1
        for row in plan["rows"]:
            record = row["_record"]
            if record["type"] != "tx":
                continue
            reason = f"Imported BIP329 label for {record['ref']}"
            for effect in row.get("tag_effects") or []:
                created, changed = _apply_bip329_tag_effect(
                    conn,
                    workspace,
                    profile,
                    effect,
                    hooks,
                    source=source,
                    reason=reason,
                )
                transaction_tags_created += 1 if created else 0
                transaction_tags_added += 1 if changed else 0
        if plan["records"]:
            hooks.invalidate_journals(conn, profile["id"])
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    return {
        "file": os.path.abspath(file_path),
        "imported": imported,
        "updated": updated,
        "records": plan["records"],
        "transaction_tags_created": transaction_tags_created,
        "transaction_tags_added": transaction_tags_added,
        "preview": _public_bip329_plan(plan, include_rows=False),
    }


def _encode_bip329_cursor(row, filters):
    token = json.dumps(
        {"created_at": row["created_at"], "filters": filters, "id": row["_id"]},
        separators=(",", ":"),
        sort_keys=True,
    )
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_bip329_cursor(cursor, filters):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        if not payload.get("created_at") or not payload.get("id"):
            raise ValueError("missing cursor fields")
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        ) from exc


def list_bip329_labels(
    conn,
    workspace_ref,
    profile_ref,
    hooks: MetadataHooks,
    cursor=None,
    limit=None,
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_RECORDS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_RECORDS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_RECORDS_LIMIT}",
            code="validation",
            hint=f"Use a smaller --limit; max page size is {MAX_RECORDS_LIMIT}.",
        )
    cursor_filters = {
        "workspace_id": workspace["id"],
        "profile_id": profile["id"],
    }
    params = [profile["id"]]
    cursor_data = _decode_bip329_cursor(cursor, cursor_filters)
    cursor_clause = ""
    if cursor_data:
        cursor_clause = "AND (created_at < ? OR (created_at = ? AND id < ?))"
        params.extend([cursor_data["created_at"], cursor_data["created_at"], cursor_data["id"]])
    params.append(effective_limit + 1)
    rows = conn.execute(
        f"""
        SELECT
            id AS _id,
            record_type AS type,
            ref,
            COALESCE(label, '') AS label,
            COALESCE(origin, '') AS origin,
            CASE
                WHEN spendable IS NULL THEN ''
                WHEN spendable = 1 THEN 'true'
                ELSE 'false'
            END AS spendable,
            created_at
        FROM bip329_labels
        WHERE profile_id = ? {cursor_clause}
        ORDER BY created_at DESC, id DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    labels = []
    for row in page:
        record = dict(row)
        record.pop("_id", None)
        labels.append(record)
    next_cursor = _encode_bip329_cursor(page[-1], cursor_filters) if has_more and page else None
    return labels, {
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def _record_from_bip329_row(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "type": row["record_type"],
        "ref": row["ref"],
        "label": row["label"],
        "origin": row["origin"],
        "spendable": None if row["spendable"] is None else bool(row["spendable"]),
        "data": _decode_bip329_data(row["data_json"]),
    }


def _payload_from_bip329_record(record: Mapping[str, Any]) -> dict[str, Any]:
    payload = {"type": record["type"], "ref": record["ref"]}
    if record.get("label") is not None:
        payload["label"] = record["label"]
    if record.get("origin") is not None:
        payload["origin"] = record["origin"]
    if record.get("spendable") is not None:
        payload["spendable"] = bool(record["spendable"])
    payload.update(record.get("data") or {})
    return payload


def _stored_bip329_records(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT record_type, ref, label, origin, spendable, data_json
        FROM bip329_labels
        WHERE profile_id = ?
        ORDER BY created_at ASC, record_type ASC, ref ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    return [_record_from_bip329_row(row) for row in rows]


def _wallet_scoped_bip329_records(
    conn: sqlite3.Connection,
    profile_id: str,
    records: Sequence[Mapping[str, Any]],
    wallet_id: str | None,
) -> list[dict[str, Any]]:
    if wallet_id is None:
        return [dict(record) for record in records]
    wallets = ownership.load_profile_wallets(conn, profile_id)
    owned_index, _warnings = ownership.build_owned_index(conn, profile_id, wallets)
    scoped = []
    for record in records:
        match_info, _tx_rows = _match_bip329_record(conn, profile_id, record, owned_index)
        if match_info.get("status") == "exact" and wallet_id in set(match_info.get("wallet_ids") or []):
            scoped.append(dict(record))
    return scoped


def _clip_bip329_label(value: str, *, max_chars: int = 255) -> str:
    text = " ".join(str(value or "").split())
    if len(text) <= max_chars:
        return text
    return text[: max_chars - 3].rstrip() + "..."


def _short_label_part(value: Any, *, max_chars: int = 72) -> str:
    return _clip_bip329_label(str(value or ""), max_chars=max_chars)


def _float_or_none(value: Any) -> float | None:
    if value in (None, ""):
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _msat_to_sats_if_whole(value: Any) -> int | None:
    try:
        msat = int(value)
    except (TypeError, ValueError):
        return None
    if msat % 1000 != 0:
        return None
    return msat // 1000


def _synthesized_tx_bip329_label(conn: sqlite3.Connection, tx: sqlite3.Row) -> dict[str, Any] | None:
    tags = _tags_for_transaction(conn, tx["id"])
    tag_labels = [tag["label"] for tag in tags]
    parts: list[str] = []
    if tx["counterparty"]:
        parts.append(_short_label_part(tx["counterparty"]))
    if tag_labels:
        parts.append("tags: " + _short_label_part(", ".join(tag_labels), max_chars=90))
    if tx["review_status"]:
        parts.append(f"review: {_short_label_part(tx['review_status'], max_chars=32)}")
    if tx["fiat_value"] is not None and tx["fiat_currency"]:
        parts.append(f"FMV {tx['fiat_currency']} {_short_label_part(tx['fiat_value'], max_chars=32)}")
    if tx["note"]:
        parts.append("note: " + _short_label_part(tx["note"], max_chars=90))
    if not parts:
        return None
    label = _clip_bip329_label(" | ".join(parts))
    payload: dict[str, Any] = {
        "type": "tx",
        "ref": tx["external_id"],
        "label": label,
        "origin": "kassiber",
        "data": {
            "kassiber": {
                "source": "synthesized",
                "transaction_id": tx["id"],
                "wallet_id": tx["wallet_id"],
                "wallet": tx["wallet_label"],
                "review_status": tx["review_status"],
                "tags": tag_labels,
            }
        },
    }
    occurred_at = tx["confirmed_at"] or tx["occurred_at"]
    if occurred_at:
        payload["data"]["time"] = occurred_at
    asset = str(tx["asset"] or "").upper()
    if asset in {"BTC", "XBT"}:
        sats = _msat_to_sats_if_whole(tx["amount"])
        if sats is not None:
            payload["data"]["value"] = -sats if tx["direction"] == "outbound" else sats
        fee_sats = _msat_to_sats_if_whole(tx["fee"])
        if fee_sats is not None and fee_sats > 0:
            payload["data"]["fee"] = fee_sats
    rate = _float_or_none(tx["fiat_rate_exact"]) if tx["fiat_rate_exact"] else _float_or_none(tx["fiat_rate"])
    if rate is not None and tx["fiat_currency"]:
        payload["data"]["rate"] = {str(tx["fiat_currency"]).upper(): rate}
    return payload


def _synthesized_bip329_records(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_id: str | None = None,
) -> list[dict[str, Any]]:
    where = ["t.profile_id = ?", "t.external_id IS NOT NULL", "t.external_id != ''"]
    params: list[Any] = [profile_id]
    if wallet_id is not None:
        where.append("t.wallet_id = ?")
        params.append(wallet_id)
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            t.wallet_id,
            w.label AS wallet_label,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_rate,
            t.fiat_rate_exact,
            t.fiat_value,
            t.review_status,
            t.counterparty,
            t.note
        FROM transactions t
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE {" AND ".join(where)}
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        params,
    ).fetchall()
    records = []
    for row in rows:
        record = _synthesized_tx_bip329_label(conn, row)
        if record is not None:
            records.append(record)
    return records


def export_bip329_labels(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: MetadataHooks,
    *,
    wallet_ref: str | None = None,
    mode: str = "stored",
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    mode = str(mode or "stored").strip().lower()
    if mode not in BIP329_EXPORT_MODES:
        raise AppError(
            f"Unsupported BIP329 export mode '{mode}'",
            code="validation",
            hint="Use stored, synthesized, or all.",
            retryable=False,
        )
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    wallet_id = wallet["id"] if wallet else None
    records: list[dict[str, Any]] = []
    exported_stored = 0
    exported_synthesized = 0
    seen: set[tuple[str, str]] = set()
    if mode in {"stored", "all"}:
        stored = _wallet_scoped_bip329_records(
            conn,
            profile["id"],
            _stored_bip329_records(conn, profile["id"]),
            wallet_id,
        )
        for record in stored:
            key = (str(record["type"]), str(record["ref"]))
            if key in seen:
                continue
            seen.add(key)
            records.append(record)
            exported_stored += 1
    if mode in {"synthesized", "all"}:
        for record in _synthesized_bip329_records(conn, profile["id"], wallet_id=wallet_id):
            key = (str(record["type"]), str(record["ref"]))
            if key in seen:
                continue
            seen.add(key)
            records.append(record)
            exported_synthesized += 1
    output_lines = [
        json.dumps(_payload_from_bip329_record(record), ensure_ascii=True)
        for record in records
    ]
    export_path = os.path.abspath(file_path)
    with open(export_path, "w", encoding="utf-8") as handle:
        if output_lines:
            handle.write("\n".join(output_lines) + "\n")
    return {
        "file": export_path,
        "exported": len(output_lines),
        "mode": mode,
        "wallet": wallet["label"] if wallet else "",
        "exported_stored": exported_stored,
        "exported_synthesized": exported_synthesized,
    }


__all__ = [
    "DEFAULT_RECORDS_LIMIT",
    "MAX_RECORDS_LIMIT",
    "MetadataHooks",
    "add_tag_to_transaction",
    "clear_transaction_note",
    "create_tag",
    "ensure_tag_row",
    "export_bip329_labels",
    "get_transaction_record",
    "import_bip329_labels",
    "list_bip329_labels",
    "preview_bip329_import",
    "list_activity_history",
    "list_transaction_history",
    "list_tags",
    "list_transaction_records",
    "remove_tag_from_transaction",
    "revert_transaction_edit",
    "set_transaction_excluded",
    "set_transaction_note",
    "stale_transaction_edit_summary",
    "update_transaction_metadata",
]
