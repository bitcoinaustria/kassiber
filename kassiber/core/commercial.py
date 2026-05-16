from __future__ import annotations

import csv
import hashlib
import json
import mimetypes
import sqlite3
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, parse_timestamp
from . import attachments as core_attachments
from . import pricing


DOCUMENT_TYPES = ("invoice", "receipt", "contract", "statement", "other")
LINK_STATES = ("suggested", "reviewed", "rejected")
CONFIDENCE_LEVELS = ("exact", "strong", "weak", "unknown")
LINK_TYPES = ("btcpay_payment_transaction", "document_btcpay", "document_transaction")
RECONCILIATION_STATES = ("unreviewed", "matched", "mismatch", "ignored")
COMMERCIAL_KINDS = ("income", "expense", "refund", "transfer", "none")
DEFAULT_PAGE_SIZE = 100
SUGGESTION_LIMIT = 500

ScopeResolver = Callable[
    [sqlite3.Connection, str | None, str | None],
    tuple[Mapping[str, Any], Mapping[str, Any]],
]
TransactionResolver = Callable[..., Mapping[str, Any]]
InvalidateJournals = Callable[[sqlite3.Connection, str], None]


@dataclass(frozen=True)
class CommercialHooks:
    resolve_scope: ScopeResolver
    resolve_transaction: TransactionResolver
    invalidate_journals: InvalidateJournals


def _now() -> str:
    return now_iso()


def _json(value: Any) -> str:
    return json.dumps(json_ready(value or {}), sort_keys=True)


def _normalize_choice(
    value: str | None,
    allowed: Sequence[str],
    *,
    label: str,
    default: str | None = None,
) -> str:
    normalized = str(value or default or "").strip().lower().replace("-", "_")
    if normalized not in allowed:
        raise AppError(
            f"Unsupported {label} '{value}'",
            code="validation",
            hint=f"Choose one of: {', '.join(allowed)}.",
        )
    return normalized


def _normalize_timestamp(value: Any) -> str | None:
    if value in (None, ""):
        return None
    parsed = parse_timestamp(value)
    return None if parsed == UNKNOWN_OCCURRED_AT else parsed


def _asset_from_payment_method(payment_method_id: str | None) -> str | None:
    raw = str(payment_method_id or "").strip().upper()
    if not raw:
        return None
    return raw.split("-", 1)[0]


def _amount_msat(value: Any) -> int | None:
    if value in (None, ""):
        return None
    return btc_to_msat(dec(value))


def _exact(value: Any) -> str | None:
    return pricing.exact_decimal(value)


def _computed_rate(fiat_value: Any, crypto_amount: Any) -> str | None:
    if fiat_value in (None, "") or crypto_amount in (None, ""):
        return None
    amount = dec(crypto_amount)
    if amount == 0:
        return None
    return _exact(dec(fiat_value) / amount)


def _stable_payment_id(payment: Mapping[str, Any]) -> str:
    raw = _json(payment.get("payment") or payment)
    return f"sha256:{hashlib.sha256(raw.encode('utf-8')).hexdigest()}"


def upsert_btcpay_provenance(
    conn: sqlite3.Connection,
    workspace: Mapping[str, Any],
    profile: Mapping[str, Any],
    *,
    backend_name: str,
    invoices: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    inserted = 0
    updated = 0
    now = _now()
    for invoice in invoices:
        invoice_id = str(invoice["invoice_id"])
        stable_key = f"btcpay:{invoice['store_id']}:invoice:{invoice_id}"
        row_id, was_insert = _upsert_record(
            conn,
            workspace,
            profile,
            backend_name=backend_name,
            store_id=invoice["store_id"],
            payment_method_id=None,
            record_type="invoice",
            stable_key=stable_key,
            invoice_id=invoice_id,
            payment_id=None,
            order_id=invoice.get("order_id"),
            status=invoice.get("status"),
            occurred_at=invoice.get("created_at"),
            asset=None,
            amount=None,
            txid=None,
            payment_hash=None,
            destination=None,
            fiat_currency=invoice.get("currency"),
            fiat_value_exact=_exact(invoice.get("amount")),
            fiat_rate_exact=None,
            pricing_timestamp=invoice.get("created_at"),
            raw_json=invoice.get("invoice"),
            now=now,
        )
        inserted += int(was_insert)
        updated += int(not was_insert)
        for payment in invoice.get("payments") or []:
            payment_id = (
                payment.get("payment_id") or payment.get("txid") or _stable_payment_id(payment)
            )
            payment_method_id = payment.get("payment_method_id")
            asset = _asset_from_payment_method(payment_method_id)
            amount_msat = _amount_msat(payment.get("amount"))
            payment_stable = f"btcpay:{invoice['store_id']}:invoice:{invoice_id}:payment:{payment_id}"
            fiat_value = payment.get("invoice_amount") or invoice.get("amount")
            rate = payment.get("rate") or _computed_rate(fiat_value, payment.get("amount"))
            _, was_insert = _upsert_record(
                conn,
                workspace,
                profile,
                backend_name=backend_name,
                store_id=invoice["store_id"],
                payment_method_id=payment_method_id,
                record_type="payment",
                stable_key=payment_stable,
                invoice_id=invoice_id,
                payment_id=payment_id,
                order_id=invoice.get("order_id"),
                status=payment.get("status") or invoice.get("status"),
                occurred_at=payment.get("received_at") or invoice.get("created_at"),
                asset=asset,
                amount=amount_msat,
                txid=payment.get("txid"),
                payment_hash=payment.get("payment_hash"),
                destination=payment.get("destination"),
                fiat_currency=payment.get("invoice_currency") or invoice.get("currency"),
                fiat_value_exact=_exact(fiat_value),
                fiat_rate_exact=_exact(rate),
                pricing_timestamp=payment.get("received_at") or invoice.get("created_at"),
                raw_json=payment.get("payment"),
                now=now,
            )
            inserted += int(was_insert)
            updated += int(not was_insert)
    conn.commit()
    return {
        "invoices_seen": len(invoices),
        "records_inserted": inserted,
        "records_updated": updated,
    }


def _upsert_record(
    conn,
    workspace,
    profile,
    *,
    backend_name,
    store_id,
    payment_method_id,
    record_type,
    stable_key,
    invoice_id,
    payment_id,
    order_id,
    status,
    occurred_at,
    asset,
    amount,
    txid,
    payment_hash,
    destination,
    fiat_currency,
    fiat_value_exact,
    fiat_rate_exact,
    pricing_timestamp,
    raw_json,
    now,
):
    existing = conn.execute(
        "SELECT id FROM btcpay_provenance_records WHERE profile_id = ? AND stable_key = ?",
        (profile["id"], stable_key),
    ).fetchone()
    if existing:
        row_id = existing["id"]
        conn.execute(
            """
            UPDATE btcpay_provenance_records
            SET backend_name = ?, store_id = ?, payment_method_id = ?, record_type = ?,
                invoice_id = ?, payment_id = ?, order_id = ?, status = ?, occurred_at = ?,
                asset = ?, amount = ?, txid = ?, payment_hash = ?, destination = ?,
                fiat_currency = ?, fiat_value_exact = ?, fiat_rate_exact = ?,
                pricing_timestamp = ?, raw_json = ?, updated_at = ?
            WHERE id = ?
            """,
            (
                backend_name,
                store_id,
                payment_method_id,
                record_type,
                invoice_id,
                payment_id,
                order_id,
                status,
                _normalize_timestamp(occurred_at),
                asset,
                amount,
                txid,
                payment_hash,
                destination,
                fiat_currency,
                fiat_value_exact,
                fiat_rate_exact,
                _normalize_timestamp(pricing_timestamp),
                _json(raw_json),
                now,
                row_id,
            ),
        )
        return row_id, False
    row_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO btcpay_provenance_records(
            id, workspace_id, profile_id, backend_name, store_id, payment_method_id,
            record_type, stable_key, invoice_id, payment_id, order_id, status,
            occurred_at, asset, amount, txid, payment_hash, destination,
            fiat_currency, fiat_value_exact, fiat_rate_exact, pricing_timestamp,
            raw_json, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            row_id,
            workspace["id"],
            profile["id"],
            backend_name,
            store_id,
            payment_method_id,
            record_type,
            stable_key,
            invoice_id,
            payment_id,
            order_id,
            status,
            _normalize_timestamp(occurred_at),
            asset,
            amount,
            txid,
            payment_hash,
            destination,
            fiat_currency,
            fiat_value_exact,
            fiat_rate_exact,
            _normalize_timestamp(pricing_timestamp),
            _json(raw_json),
            now,
            now,
        ),
    )
    return row_id, True


def list_btcpay_records(
    conn,
    workspace_ref,
    profile_ref,
    hooks: CommercialHooks,
    *,
    record_type=None,
    limit=100,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    where = ["profile_id = ?"]
    params: list[Any] = [profile["id"]]
    if record_type:
        where.append("record_type = ?")
        params.append(_normalize_choice(record_type, ("invoice", "payment"), label="record type"))
    params.append(max(1, min(int(limit or 100), 1000)))
    rows = conn.execute(
        f"""
        SELECT *
        FROM btcpay_provenance_records
        WHERE {' AND '.join(where)}
        ORDER BY COALESCE(occurred_at, created_at) DESC, id DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    return [_record_payload(row) for row in rows]


def create_document(
    conn,
    workspace_ref,
    profile_ref,
    hooks: CommercialHooks,
    *,
    document_type,
    label,
    external_ref=None,
    issuer=None,
    counterparty=None,
    issued_at=None,
    due_at=None,
    fiat_currency=None,
    fiat_value=None,
    notes=None,
    raw_json=None,
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    doc_type = _normalize_choice(document_type, DOCUMENT_TYPES, label="document type")
    now = _now()
    document_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO external_documents(
            id, workspace_id, profile_id, document_type, label, external_ref,
            issuer, counterparty, issued_at, due_at, fiat_currency, fiat_value_exact,
            notes, raw_json, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            document_id,
            workspace["id"],
            profile["id"],
            doc_type,
            label,
            external_ref,
            issuer,
            counterparty,
            _normalize_timestamp(issued_at),
            _normalize_timestamp(due_at),
            str(fiat_currency).upper() if fiat_currency else None,
            _exact(fiat_value),
            notes,
            _json(raw_json),
            now,
            now,
        ),
    )
    conn.commit()
    return get_document(conn, profile["id"], document_id)


def get_document(conn, profile_id, document_ref):
    row = conn.execute(
        """
        SELECT *
        FROM external_documents
        WHERE profile_id = ? AND (id = ? OR external_ref = ? OR label = ?)
        ORDER BY created_at DESC
        LIMIT 1
        """,
        (profile_id, document_ref, document_ref, document_ref),
    ).fetchone()
    if not row:
        raise AppError(f"External document '{document_ref}' not found", code="not_found")
    return _document_payload(row)


def list_documents(conn, workspace_ref, profile_ref, hooks: CommercialHooks, *, limit=100):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT *
        FROM external_documents
        WHERE profile_id = ?
        ORDER BY created_at DESC, id DESC
        LIMIT ?
        """,
        (profile["id"], max(1, min(int(limit or 100), 1000))),
    ).fetchall()
    return [_document_payload(row) for row in rows]


def attach_document_evidence(
    conn,
    data_root,
    workspace_ref,
    profile_ref,
    document_ref,
    hooks: CommercialHooks,
    *,
    file_path=None,
    url=None,
    label=None,
    media_type=None,
):
    if bool(file_path) == bool(url):
        raise AppError("Provide exactly one of --file or --url", code="validation")
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    document = get_document(conn, profile["id"], document_ref)
    attachment_id = str(uuid.uuid4())
    created_at = _now()
    attachment_type = "url" if url else "file"
    original_filename = None
    stored_relpath = None
    source_url = None
    size_bytes = None
    sha256 = None
    destination = None
    attachments_root = core_attachments._attachments_root(data_root)
    if file_path:
        source = Path(file_path).expanduser()
        if not source.is_file():
            raise AppError(f"Attachment file '{file_path}' not found", code="not_found")
        original_filename = source.name
        destination, stored_relpath = core_attachments._attachment_storage_path(
            attachments_root,
            profile["id"],
            attachment_id,
            original_filename,
        )
        size_bytes, sha256 = core_attachments._hash_and_copy_file(source, destination)
        media_type = media_type or mimetypes.guess_type(source.name)[0] or "application/octet-stream"
        label = label or original_filename
    else:
        source_url = str(url)
        label = label or source_url
        media_type = media_type or "text/uri-list"
    try:
        conn.execute(
            """
            INSERT INTO attachments(
                id, workspace_id, profile_id, transaction_id, attachment_type, label,
                original_filename, stored_relpath, source_url, media_type,
                size_bytes, sha256, created_at
            ) VALUES(?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                attachment_id,
                workspace["id"],
                profile["id"],
                attachment_type,
                label,
                original_filename,
                stored_relpath,
                source_url,
                media_type,
                size_bytes,
                sha256,
                created_at,
            ),
        )
        conn.execute(
            """
            INSERT INTO external_document_attachments(document_id, attachment_id, created_at)
            VALUES(?, ?, ?)
            """,
            (document["id"], attachment_id, created_at),
        )
        conn.commit()
    except Exception:
        if destination is not None:
            try:
                destination.unlink()
            except OSError:
                pass
        raise
    return {"document_id": document["id"], "attachment_id": attachment_id, "label": label}


def suggest_links(
    conn,
    workspace_ref,
    profile_ref,
    hooks: CommercialHooks,
    *,
    limit=SUGGESTION_LIMIT,
):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT p.*, t.id AS transaction_id, t.amount AS transaction_amount,
               t.fiat_value_exact AS transaction_fiat_value_exact,
               t.fiat_value AS transaction_fiat_value
        FROM btcpay_provenance_records p
        JOIN transactions t
          ON t.profile_id = p.profile_id
         AND p.record_type = 'payment'
         AND (
              (p.txid IS NOT NULL AND p.txid != '' AND t.external_id = p.txid)
              OR (p.payment_hash IS NOT NULL AND p.payment_hash != '' AND t.payment_hash = p.payment_hash)
         )
        WHERE p.profile_id = ?
        ORDER BY COALESCE(p.occurred_at, p.created_at) DESC
        LIMIT ?
        """,
        (profile["id"], max(1, min(int(limit or SUGGESTION_LIMIT), SUGGESTION_LIMIT))),
    ).fetchall()
    created = 0
    suggestions = []
    now = _now()
    for row in rows:
        confidence = (
            "exact"
            if row["amount"] is not None
            and int(row["amount"]) == abs(int(row["transaction_amount"]))
            else "strong"
        )
        link = _upsert_link(
            conn,
            workspace,
            profile,
            btcpay_record_id=row["id"],
            document_id=None,
            transaction_id=row["transaction_id"],
            link_type="btcpay_payment_transaction",
            state="suggested",
            confidence=confidence,
            method="txid_or_payment_hash",
            allocation_amount=row["amount"],
            allocation_fiat_exact=row["fiat_value_exact"],
            reconciliation_state="unreviewed",
            commercial_kind=None,
            notes=None,
            now=now,
        )
        created += int(link["created"])
        suggestions.append(link["link"])
    document_links = _suggest_document_links(conn, workspace, profile, now)
    suggestions.extend(document_links["links"])
    created += document_links["created"]
    document_transaction_links = _suggest_document_transaction_links(conn, workspace, profile, now)
    suggestions.extend(document_transaction_links["links"])
    created += document_transaction_links["created"]
    conn.commit()
    return {"created": created, "suggestions": suggestions}


def _suggest_document_links(conn, workspace, profile, now):
    rows = conn.execute(
        """
        SELECT d.id AS document_id, p.id AS btcpay_record_id, d.fiat_value_exact AS doc_value,
               p.fiat_value_exact AS btcpay_value
        FROM external_documents d
        JOIN btcpay_provenance_records p
          ON p.profile_id = d.profile_id
         AND p.record_type = 'invoice'
         AND (
              (d.external_ref IS NOT NULL AND d.external_ref != '' AND (d.external_ref = p.invoice_id OR d.external_ref = p.order_id))
              OR (
                   d.fiat_currency IS NOT NULL
                   AND p.fiat_currency = d.fiat_currency
                   AND d.fiat_value_exact IS NOT NULL
                   AND p.fiat_value_exact = d.fiat_value_exact
              )
         )
        WHERE d.profile_id = ?
        LIMIT ?
        """,
        (profile["id"], SUGGESTION_LIMIT),
    ).fetchall()
    created = 0
    links = []
    for row in rows:
        link = _upsert_link(
            conn,
            workspace,
            profile,
            btcpay_record_id=row["btcpay_record_id"],
            document_id=row["document_id"],
            transaction_id=None,
            link_type="document_btcpay",
            state="suggested",
            confidence="strong",
            method="document_reference_or_amount",
            allocation_amount=None,
            allocation_fiat_exact=row["doc_value"] or row["btcpay_value"],
            reconciliation_state="unreviewed",
            commercial_kind=None,
            notes=None,
            now=now,
        )
        created += int(link["created"])
        links.append(link["link"])
    return {"created": created, "links": links}


def _suggest_document_transaction_links(conn, workspace, profile, now):
    rows = conn.execute(
        """
        SELECT d.id AS document_id, p.id AS btcpay_record_id,
               t.id AS transaction_id, d.fiat_value_exact AS doc_value,
               p.fiat_value_exact AS btcpay_value, p.amount AS payment_amount
        FROM external_documents d
        JOIN btcpay_provenance_records p
          ON p.profile_id = d.profile_id
         AND p.record_type = 'payment'
         AND (
              (d.external_ref IS NOT NULL AND d.external_ref != '' AND (d.external_ref = p.invoice_id OR d.external_ref = p.order_id))
              OR (
                   d.fiat_currency IS NOT NULL
                   AND p.fiat_currency = d.fiat_currency
                   AND d.fiat_value_exact IS NOT NULL
                   AND p.fiat_value_exact = d.fiat_value_exact
              )
         )
        JOIN transactions t
          ON t.profile_id = p.profile_id
         AND (
              (p.txid IS NOT NULL AND p.txid != '' AND t.external_id = p.txid)
              OR (p.payment_hash IS NOT NULL AND p.payment_hash != '' AND t.payment_hash = p.payment_hash)
         )
        WHERE d.profile_id = ?
        LIMIT ?
        """,
        (profile["id"], SUGGESTION_LIMIT),
    ).fetchall()
    created = 0
    links = []
    for row in rows:
        link = _upsert_link(
            conn,
            workspace,
            profile,
            btcpay_record_id=row["btcpay_record_id"],
            document_id=row["document_id"],
            transaction_id=row["transaction_id"],
            link_type="btcpay_payment_transaction",
            state="suggested",
            confidence="strong",
            method="document_btcpay_txid_or_payment_hash",
            allocation_amount=row["payment_amount"],
            allocation_fiat_exact=row["doc_value"] or row["btcpay_value"],
            reconciliation_state="unreviewed",
            commercial_kind=None,
            notes=None,
            now=now,
        )
        created += int(link["created"])
        links.append(link["link"])
    return {"created": created, "links": links}


def _upsert_link(conn, workspace, profile, **values):
    existing = conn.execute(
        """
        SELECT id FROM commercial_links
        WHERE profile_id = ?
          AND COALESCE(btcpay_record_id, '') = COALESCE(?, '')
          AND COALESCE(document_id, '') = COALESCE(?, '')
          AND COALESCE(transaction_id, '') = COALESCE(?, '')
          AND link_type = ?
          AND state != 'rejected'
        """,
        (
            profile["id"],
            values.get("btcpay_record_id"),
            values.get("document_id"),
            values.get("transaction_id"),
            values["link_type"],
        ),
    ).fetchone()
    if existing:
        return {"created": False, "link": get_link(conn, profile["id"], existing["id"])}
    link_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO commercial_links(
            id, workspace_id, profile_id, btcpay_record_id, document_id,
            transaction_id, link_type, state, confidence, method,
            allocation_amount, allocation_fiat_exact, reconciliation_state,
            commercial_kind, notes, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            link_id,
            workspace["id"],
            profile["id"],
            values.get("btcpay_record_id"),
            values.get("document_id"),
            values.get("transaction_id"),
            values["link_type"],
            values["state"],
            values["confidence"],
            values["method"],
            values.get("allocation_amount"),
            values.get("allocation_fiat_exact"),
            values["reconciliation_state"],
            values.get("commercial_kind"),
            values.get("notes"),
            values["now"],
            values["now"],
        ),
    )
    return {"created": True, "link": get_link(conn, profile["id"], link_id)}


def list_links(conn, workspace_ref, profile_ref, hooks: CommercialHooks, *, state=None, limit=100):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    where = ["cl.profile_id = ?"]
    params: list[Any] = [profile["id"]]
    if state:
        where.append("cl.state = ?")
        params.append(_normalize_choice(state, LINK_STATES, label="link state"))
    params.append(max(1, min(int(limit or 100), 1000)))
    rows = conn.execute(
        f"""
        SELECT cl.*, p.stable_key AS btcpay_stable_key, p.invoice_id, p.payment_id,
               p.txid, d.label AS document_label, t.external_id AS transaction_external_id
        FROM commercial_links cl
        LEFT JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
        LEFT JOIN external_documents d ON d.id = cl.document_id
        LEFT JOIN transactions t ON t.id = cl.transaction_id
        WHERE {' AND '.join(where)}
        ORDER BY cl.created_at DESC, cl.id DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    return [_link_payload(row) for row in rows]


def get_link(conn, profile_id, link_ref):
    row = conn.execute(
        """
        SELECT cl.*, p.stable_key AS btcpay_stable_key, p.invoice_id, p.payment_id,
               p.txid, d.label AS document_label, t.external_id AS transaction_external_id
        FROM commercial_links cl
        LEFT JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
        LEFT JOIN external_documents d ON d.id = cl.document_id
        LEFT JOIN transactions t ON t.id = cl.transaction_id
        WHERE cl.profile_id = ? AND cl.id = ?
        """,
        (profile_id, link_ref),
    ).fetchone()
    if not row:
        raise AppError(f"Commercial link '{link_ref}' not found", code="not_found")
    return _link_payload(row)


def review_link(
    conn,
    workspace_ref,
    profile_ref,
    link_ref,
    hooks: CommercialHooks,
    *,
    state,
    reconciliation_state=None,
    commercial_kind=None,
    notes=None,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    state = _normalize_choice(state, LINK_STATES, label="link state")
    reconciliation = _normalize_choice(
        reconciliation_state,
        RECONCILIATION_STATES,
        label="reconciliation state",
        default="matched" if state == "reviewed" else "ignored",
    )
    commercial = None
    if commercial_kind is not None:
        commercial = _normalize_choice(commercial_kind, COMMERCIAL_KINDS, label="commercial kind")
        if commercial == "none":
            commercial = None
    now = _now()
    cursor = conn.execute(
        """
        UPDATE commercial_links
        SET state = ?, reconciliation_state = ?, commercial_kind = COALESCE(?, commercial_kind),
            notes = COALESCE(?, notes), reviewed_at = ?, updated_at = ?
        WHERE profile_id = ? AND id = ?
        """,
        (state, reconciliation, commercial, notes, now, now, profile["id"], link_ref),
    )
    if cursor.rowcount == 0:
        raise AppError(f"Commercial link '{link_ref}' not found", code="not_found")
    link = get_link(conn, profile["id"], link_ref)
    applied = False
    if state == "reviewed" and link.get("transaction_id"):
        applied = _apply_reviewed_link_to_transaction(conn, profile, link, commercial)
        if applied:
            hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    link = get_link(conn, profile["id"], link_ref)
    link["applied_to_transaction"] = applied
    return link


def _apply_reviewed_link_to_transaction(conn, profile, link, commercial_kind):
    record = None
    if link.get("btcpay_record_id"):
        record = conn.execute(
            "SELECT * FROM btcpay_provenance_records WHERE id = ?",
            (link["btcpay_record_id"],),
        ).fetchone()
    if not record:
        return False
    updates: dict[str, Any] = {}
    source_kind = pricing.SOURCE_BTCPAY_PAYMENT if record["record_type"] == "payment" else pricing.SOURCE_BTCPAY_INVOICE
    amount = abs(int(record["amount"] or 0))
    fiat_value = record["fiat_value_exact"]
    rate = record["fiat_rate_exact"]
    if not rate and fiat_value and amount > 0:
        rate = _exact(dec(fiat_value) / msat_to_btc(amount))
    if fiat_value or rate:
        payload = pricing.pricing_payload(
            rate=rate,
            value=fiat_value,
            source_kind=source_kind,
            quality=pricing.QUALITY_EXACT,
            provider="btcpay",
            pair=f"{record['asset'] or 'BTC'}-{record['fiat_currency']}" if record["fiat_currency"] else None,
            pricing_timestamp=record["pricing_timestamp"] or record["occurred_at"],
            fetched_at=record["updated_at"],
            granularity="invoice_payment",
            method="reviewed_commercial_link",
            external_ref=record["stable_key"],
        )
        updates.update(payload)
    if commercial_kind and commercial_kind != "transfer":
        updates["kind"] = commercial_kind
    if not updates:
        return False
    assignments = ", ".join(f"{column} = ?" for column in updates)
    conn.execute(
        f"UPDATE transactions SET {assignments} WHERE profile_id = ? AND id = ?",
        (*updates.values(), profile["id"], link["transaction_id"]),
    )
    return True


def build_reviewed_subledger_rows(conn, workspace_ref, profile_ref, hooks: CommercialHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT cl.id AS link_id, cl.commercial_kind, cl.allocation_amount,
               cl.allocation_fiat_exact, cl.reconciliation_state,
               t.id AS transaction_id, t.external_id, t.occurred_at, t.direction,
               t.asset, t.amount, t.fee, t.fiat_currency, t.fiat_value_exact,
               t.pricing_source_kind, w.label AS wallet,
               p.invoice_id, p.payment_id, p.order_id, p.stable_key,
               d.id AS document_id, d.label AS document_label, d.external_ref AS document_external_ref
        FROM commercial_links cl
        JOIN transactions t ON t.id = cl.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
        LEFT JOIN external_documents d ON d.id = cl.document_id
        WHERE cl.profile_id = ? AND cl.state = 'reviewed'
        ORDER BY t.occurred_at ASC, cl.created_at ASC
        """,
        (profile["id"],),
    ).fetchall()
    return [_subledger_payload(row) for row in rows]


def export_reviewed_subledger_csv(conn, workspace_ref, profile_ref, file_path, hooks: CommercialHooks):
    rows = build_reviewed_subledger_rows(conn, workspace_ref, profile_ref, hooks)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    headers = [
        "occurred_at",
        "wallet",
        "external_id",
        "direction",
        "asset",
        "amount",
        "fee_msat",
        "fiat_currency",
        "fiat_value_exact",
        "pricing_source_kind",
        "commercial_kind",
        "invoice_id",
        "payment_id",
        "order_id",
        "document_label",
        "document_external_ref",
        "reconciliation_state",
    ]
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=headers)
        writer.writeheader()
        for row in rows:
            writer.writerow({key: row.get(key, "") for key in headers})
    return {"file": str(path), "rows": len(rows)}


def _record_payload(row):
    return {
        "id": row["id"],
        "record_type": row["record_type"],
        "stable_key": row["stable_key"],
        "backend": row["backend_name"] or "",
        "store_id": row["store_id"],
        "payment_method_id": row["payment_method_id"] or "",
        "invoice_id": row["invoice_id"] or "",
        "payment_id": row["payment_id"] or "",
        "order_id": row["order_id"] or "",
        "status": row["status"] or "",
        "occurred_at": row["occurred_at"] or "",
        "asset": row["asset"] or "",
        "amount_msat": row["amount"],
        "amount": float(msat_to_btc(row["amount"])) if row["amount"] is not None else None,
        "txid": row["txid"] or "",
        "payment_hash": row["payment_hash"] or "",
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value_exact": row["fiat_value_exact"] or "",
        "fiat_rate_exact": row["fiat_rate_exact"] or "",
        "pricing_timestamp": row["pricing_timestamp"] or "",
        "updated_at": row["updated_at"],
    }


def _document_payload(row):
    return {
        "id": row["id"],
        "document_type": row["document_type"],
        "label": row["label"],
        "external_ref": row["external_ref"] or "",
        "issuer": row["issuer"] or "",
        "counterparty": row["counterparty"] or "",
        "issued_at": row["issued_at"] or "",
        "due_at": row["due_at"] or "",
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value_exact": row["fiat_value_exact"] or "",
        "review_state": row["review_state"],
        "notes": row["notes"] or "",
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def _link_payload(row):
    return {
        "id": row["id"],
        "btcpay_record_id": row["btcpay_record_id"] or "",
        "btcpay_stable_key": row["btcpay_stable_key"] or "",
        "invoice_id": row["invoice_id"] or "",
        "payment_id": row["payment_id"] or "",
        "txid": row["txid"] or "",
        "document_id": row["document_id"] or "",
        "document_label": row["document_label"] or "",
        "transaction_id": row["transaction_id"] or "",
        "transaction_external_id": row["transaction_external_id"] or "",
        "link_type": row["link_type"],
        "state": row["state"],
        "confidence": row["confidence"],
        "method": row["method"],
        "allocation_amount_msat": row["allocation_amount"],
        "allocation_amount": float(msat_to_btc(row["allocation_amount"])) if row["allocation_amount"] is not None else None,
        "allocation_fiat_exact": row["allocation_fiat_exact"] or "",
        "reconciliation_state": row["reconciliation_state"],
        "commercial_kind": row["commercial_kind"] or "",
        "notes": row["notes"] or "",
        "reviewed_at": row["reviewed_at"] or "",
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def _subledger_payload(row):
    amount_msat = int(row["amount"])
    return {
        "transaction_id": row["transaction_id"],
        "external_id": row["external_id"] or "",
        "occurred_at": row["occurred_at"],
        "wallet": row["wallet"],
        "direction": row["direction"],
        "asset": row["asset"],
        "amount_msat": amount_msat,
        "amount": float(msat_to_btc(amount_msat)),
        "fee_msat": int(row["fee"]),
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value_exact": row["fiat_value_exact"] or "",
        "pricing_source_kind": row["pricing_source_kind"] or "",
        "commercial_kind": row["commercial_kind"] or "",
        "reconciliation_state": row["reconciliation_state"],
        "invoice_id": row["invoice_id"] or "",
        "payment_id": row["payment_id"] or "",
        "order_id": row["order_id"] or "",
        "btcpay_ref": row["stable_key"] or "",
        "document_id": row["document_id"] or "",
        "document_label": row["document_label"] or "",
        "document_external_ref": row["document_external_ref"] or "",
    }


__all__ = [
    "COMMERCIAL_KINDS",
    "CONFIDENCE_LEVELS",
    "DOCUMENT_TYPES",
    "LINK_STATES",
    "CommercialHooks",
    "attach_document_evidence",
    "build_reviewed_subledger_rows",
    "create_document",
    "export_reviewed_subledger_csv",
    "list_btcpay_records",
    "list_documents",
    "list_links",
    "review_link",
    "suggest_links",
    "upsert_btcpay_provenance",
]
