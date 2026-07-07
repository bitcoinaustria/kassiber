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
from urllib.parse import urlparse

from ..envelope import json_ready
from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, parse_timestamp
from . import attachments as core_attachments
from . import pricing


DOCUMENT_TYPES = ("invoice", "receipt", "contract", "statement", "other")
LINK_STATES = ("suggested", "reviewed", "rejected")
CONFIDENCE_LEVELS = ("exact", "strong", "weak", "unknown")
RECONCILIATION_STATES = ("unreviewed", "matched", "mismatch", "ignored")
COMMERCIAL_KINDS = ("income", "expense", "refund", "transfer", "none")
DEFAULT_PAGE_SIZE = 100
SUGGESTION_LIMIT = 500
TRANSACTION_APPLY_COLUMNS = (
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
    "kind",
    "commercial_applied_link_id",
)

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
            payment_request_id=invoice.get("payment_request_id"),
            origin_kind=invoice.get("origin_kind"),
            origin_app_id=invoice.get("origin_app_id"),
            origin_label=invoice.get("origin_label"),
            origin_url=invoice.get("origin_url") or invoice.get("order_url"),
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
                payment_request_id=invoice.get("payment_request_id"),
                origin_kind=invoice.get("origin_kind"),
                origin_app_id=invoice.get("origin_app_id"),
                origin_label=invoice.get("origin_label"),
                origin_url=invoice.get("origin_url") or invoice.get("order_url"),
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
    payment_request_id,
    origin_kind,
    origin_app_id,
    origin_label,
    origin_url,
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
                payment_request_id = ?, origin_kind = ?, origin_app_id = ?,
                origin_label = ?, origin_url = ?, fiat_currency = ?,
                fiat_value_exact = ?, fiat_rate_exact = ?,
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
                payment_request_id,
                origin_kind,
                origin_app_id,
                origin_label,
                origin_url,
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
            payment_request_id, origin_kind, origin_app_id, origin_label, origin_url,
            fiat_currency, fiat_value_exact, fiat_rate_exact, pricing_timestamp,
            raw_json, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            payment_request_id,
            origin_kind,
            origin_app_id,
            origin_label,
            origin_url,
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
    if external_ref:
        existing = conn.execute(
            """
            SELECT id FROM external_documents
            WHERE profile_id = ? AND external_ref = ?
            """,
            (profile["id"], external_ref),
        ).fetchone()
        if existing:
            raise AppError(
                f"External document reference '{external_ref}' already exists",
                code="conflict",
                hint="Use the existing document id or choose a unique external reference.",
            )
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
        "SELECT * FROM external_documents WHERE profile_id = ? AND id = ?",
        (profile_id, document_ref),
    ).fetchone()
    if row:
        return _document_payload(row)
    rows = conn.execute(
        """
        SELECT *
        FROM external_documents
        WHERE profile_id = ? AND (external_ref = ? OR label = ?)
        ORDER BY created_at DESC
        """,
        (profile_id, document_ref, document_ref),
    ).fetchall()
    if not rows:
        raise AppError(f"External document '{document_ref}' not found", code="not_found")
    if len(rows) > 1:
        raise AppError(
            f"External document reference '{document_ref}' is ambiguous",
            code="ambiguous",
            hint="Use the document id instead of a label or external reference.",
            details={"matches": len(rows)},
        )
    return _document_payload(rows[0])


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
                # Preserve the original database error; file cleanup is best-effort.
                pass
        raise
    return {"document_id": document["id"], "attachment_id": attachment_id, "label": label}


def _matching_transactions_for_record(conn, profile_id, record):
    clauses = []
    params: list[Any] = [profile_id]
    if record["txid"]:
        clauses.append("external_id = ?")
        params.append(record["txid"])
    if record["payment_hash"]:
        clauses.append("payment_hash = ?")
        params.append(record["payment_hash"])
    if not clauses:
        return []
    return conn.execute(
        f"""
        SELECT id, amount, fiat_value_exact, fiat_value
        FROM transactions
        WHERE profile_id = ? AND ({' OR '.join(clauses)})
        ORDER BY occurred_at ASC, id ASC
        """,
        params,
    ).fetchall()


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
        SELECT p.*
        FROM btcpay_provenance_records p
        WHERE p.profile_id = ? AND p.record_type = 'payment'
        ORDER BY COALESCE(p.occurred_at, p.created_at) DESC
        LIMIT ?
        """,
        (profile["id"], max(1, min(int(limit or SUGGESTION_LIMIT), SUGGESTION_LIMIT))),
    ).fetchall()
    created = 0
    suggestions = []
    now = _now()
    for row in rows:
        matches = _matching_transactions_for_record(conn, profile["id"], row)
        if len(matches) != 1:
            continue
        tx = matches[0]
        confidence = (
            "exact"
            if row["amount"] is not None
            and int(row["amount"]) == abs(int(tx["amount"]))
            else "strong"
        )
        link = _upsert_link(
            conn,
            workspace,
            profile,
            btcpay_record_id=row["id"],
            document_id=None,
            transaction_id=tx["id"],
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
    unique_suggestions = {}
    for suggestion in suggestions:
        unique_suggestions[suggestion["id"]] = suggestion
    return {"created": created, "suggestions": list(unique_suggestions.values())}


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
              (
                   d.external_ref IS NOT NULL
                   AND d.external_ref != ''
                   AND (
                       d.external_ref = p.invoice_id
                       OR d.external_ref = p.order_id
                       OR d.external_ref = p.payment_request_id
                   )
              )
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
               p.txid, p.payment_hash,
               d.fiat_value_exact AS doc_value,
               p.fiat_value_exact AS btcpay_value, p.amount AS payment_amount
        FROM external_documents d
        JOIN btcpay_provenance_records p
          ON p.profile_id = d.profile_id
         AND p.record_type = 'payment'
         AND (
              (
                   d.external_ref IS NOT NULL
                   AND d.external_ref != ''
                   AND (
                       d.external_ref = p.invoice_id
                       OR d.external_ref = p.order_id
                       OR d.external_ref = p.payment_request_id
                   )
              )
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
        matches = _matching_transactions_for_record(conn, profile["id"], row)
        if len(matches) != 1:
            continue
        tx = matches[0]
        link = _upsert_link(
            conn,
            workspace,
            profile,
            btcpay_record_id=row["btcpay_record_id"],
            document_id=row["document_id"],
            transaction_id=tx["id"],
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
    if values["link_type"] == "btcpay_payment_transaction":
        existing = conn.execute(
            """
            SELECT id, document_id FROM commercial_links
            WHERE profile_id = ?
              AND COALESCE(btcpay_record_id, '') = COALESCE(?, '')
              AND COALESCE(transaction_id, '') = COALESCE(?, '')
              AND link_type = ?
              AND state != 'rejected'
            """,
            (
                profile["id"],
                values.get("btcpay_record_id"),
                values.get("transaction_id"),
                values["link_type"],
            ),
        ).fetchone()
    else:
        existing = conn.execute(
            """
            SELECT id, document_id FROM commercial_links
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
        if (
            values["link_type"] == "btcpay_payment_transaction"
            and values.get("document_id")
            and not existing["document_id"]
        ):
            conn.execute(
                """
                UPDATE commercial_links
                SET document_id = ?, allocation_fiat_exact = COALESCE(?, allocation_fiat_exact),
                    method = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    values.get("document_id"),
                    values.get("allocation_fiat_exact"),
                    values["method"],
                    values["now"],
                    existing["id"],
                ),
            )
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
               p.txid, p.payment_request_id, p.origin_kind, p.origin_label,
               d.label AS document_label, t.external_id AS transaction_external_id
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


def get_transaction_commercial_context(
    conn,
    workspace_ref,
    profile_ref,
    transaction_ref,
    hooks: CommercialHooks,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], transaction_ref)
    rows = conn.execute(
        """
        SELECT cl.*, p.invoice_id, p.payment_id,
               d.label AS document_label,
               p.id AS payment_record_id, p.record_type AS payment_record_type,
               p.invoice_id AS payment_invoice_id,
               p.payment_id AS payment_payment_id, p.order_id AS payment_order_id,
               p.status AS payment_status, p.occurred_at AS payment_occurred_at,
               p.asset AS payment_asset, p.amount AS payment_amount,
               p.payment_request_id AS payment_payment_request_id,
               p.origin_kind AS payment_origin_kind,
               p.origin_app_id AS payment_origin_app_id,
               p.origin_label AS payment_origin_label,
               p.origin_url AS payment_origin_url,
               p.fiat_currency AS payment_fiat_currency,
               p.fiat_value_exact AS payment_fiat_value_exact,
               p.fiat_rate_exact AS payment_fiat_rate_exact,
               p.pricing_timestamp AS payment_pricing_timestamp,
               p.updated_at AS payment_updated_at,
               inv.id AS invoice_record_id, inv.record_type AS invoice_record_type,
               inv.invoice_id AS invoice_invoice_id,
               inv.payment_id AS invoice_payment_id,
               inv.order_id AS invoice_order_id,
               inv.status AS invoice_status,
               inv.occurred_at AS invoice_occurred_at,
               inv.asset AS invoice_asset,
               inv.amount AS invoice_amount,
               inv.payment_request_id AS invoice_payment_request_id,
               inv.origin_kind AS invoice_origin_kind,
               inv.origin_app_id AS invoice_origin_app_id,
               inv.origin_label AS invoice_origin_label,
               inv.origin_url AS invoice_origin_url,
               inv.fiat_currency AS invoice_fiat_currency,
               inv.fiat_value_exact AS invoice_fiat_value_exact,
               inv.fiat_rate_exact AS invoice_fiat_rate_exact,
               inv.pricing_timestamp AS invoice_pricing_timestamp,
               inv.updated_at AS invoice_updated_at,
               d.id AS ctx_document_id,
               d.document_type AS ctx_document_type,
               d.label AS ctx_document_label,
               d.external_ref AS ctx_document_external_ref,
               d.review_state AS ctx_document_review_state
        FROM commercial_links cl
        LEFT JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
        LEFT JOIN btcpay_provenance_records inv
          ON inv.profile_id = cl.profile_id
         AND inv.store_id = p.store_id
         AND inv.invoice_id = p.invoice_id
         AND inv.record_type = 'invoice'
        LEFT JOIN external_documents d ON d.id = cl.document_id
        WHERE cl.profile_id = ? AND cl.transaction_id = ?
          AND cl.state IN ('suggested', 'reviewed')
        ORDER BY cl.reviewed_at DESC, cl.created_at DESC, cl.id DESC
        """,
        (profile["id"], tx["id"]),
    ).fetchall()
    documents: dict[str, dict[str, Any]] = {}
    btcpay_matches = []
    links = []
    for row in rows:
        link = _link_context_payload(row)
        links.append(link)
        if row["ctx_document_id"]:
            documents[row["ctx_document_id"]] = _document_context_payload(row)
        payment = _btcpay_record_context_payload(row, "payment")
        invoice = _btcpay_record_context_payload(row, "invoice")
        if payment or invoice:
            btcpay_matches.append(
                {
                    "link": link,
                    "payment": payment,
                    "invoice": invoice,
                    "payment_request": _payment_request_context(payment, invoice),
                    "origin": _origin_context(payment, invoice),
                }
            )
    return {
        "transaction_id": tx["id"],
        "transaction_external_id": tx["external_id"] or "",
        "links": links,
        "btcpay": btcpay_matches,
        "documents": list(documents.values()),
    }


def get_link(conn, profile_id, link_ref):
    row = conn.execute(
        """
        SELECT cl.*, p.stable_key AS btcpay_stable_key, p.invoice_id, p.payment_id,
               p.txid, p.payment_request_id, p.origin_kind, p.origin_label,
               d.label AS document_label, t.external_id AS transaction_external_id
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


def _get_link_row(conn, profile_id, link_ref):
    row = conn.execute(
        "SELECT * FROM commercial_links WHERE profile_id = ? AND id = ?",
        (profile_id, link_ref),
    ).fetchone()
    if not row:
        raise AppError(f"Commercial link '{link_ref}' not found", code="not_found")
    return row


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
    link_before = _get_link_row(conn, profile["id"], link_ref)
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
    else:
        commercial = link_before["commercial_kind"]
    now = _now()
    applied = False
    restored = False
    apply_snapshot_json = link_before["applied_transaction_snapshot_json"]
    reviewed_snapshot_json = link_before["reviewed_record_snapshot_json"]
    reviewed_snapshot_sha256 = link_before["reviewed_record_snapshot_sha256"]
    if link_before["state"] == "reviewed" and state != "reviewed" and link_before["transaction_id"]:
        restored = _restore_reviewed_link_transaction(conn, profile, link_before)
    if state == "reviewed" and not reviewed_snapshot_sha256:
        reviewed_snapshot_json, reviewed_snapshot_sha256 = _reviewed_record_snapshot_for_link(
            conn, link_before
        )
    if state == "reviewed" and link_before["transaction_id"]:
        apply_result = _apply_reviewed_link_to_transaction(
            conn, profile, link_before, commercial
        )
        applied = apply_result["applied"]
        apply_snapshot_json = apply_result["snapshot_json"]
    reviewed_at = now if state == "reviewed" else link_before["reviewed_at"]
    conn.execute(
        """
        UPDATE commercial_links
        SET state = ?, reconciliation_state = ?, commercial_kind = ?,
            applied_transaction_snapshot_json = ?,
            reviewed_record_snapshot_json = ?,
            reviewed_record_snapshot_sha256 = ?,
            notes = COALESCE(?, notes), reviewed_at = ?, updated_at = ?
        WHERE profile_id = ? AND id = ?
        """,
        (
            state,
            reconciliation,
            commercial,
            apply_snapshot_json,
            reviewed_snapshot_json,
            reviewed_snapshot_sha256,
            notes,
            reviewed_at,
            now,
            profile["id"],
            link_ref,
        ),
    )
    if applied or restored:
        hooks.invalidate_journals(conn, profile["id"])
    conn.commit()
    link = get_link(conn, profile["id"], link_ref)
    link["applied_to_transaction"] = applied
    link["restored_transaction"] = restored
    return link


def _reviewed_record_snapshot_for_link(conn, link):
    if not link["btcpay_record_id"]:
        return link["reviewed_record_snapshot_json"], link["reviewed_record_snapshot_sha256"]
    record = conn.execute(
        "SELECT raw_json FROM btcpay_provenance_records WHERE id = ?",
        (link["btcpay_record_id"],),
    ).fetchone()
    if not record:
        return None, None
    raw_json = record["raw_json"] or "{}"
    return raw_json, hashlib.sha256(raw_json.encode("utf-8")).hexdigest()


def _transaction_snapshot(row):
    return _json({column: row[column] for column in TRANSACTION_APPLY_COLUMNS})


def _restore_reviewed_link_transaction(conn, profile, link):
    snapshot_json = link["applied_transaction_snapshot_json"]
    if not snapshot_json:
        raise AppError(
            f"Commercial link '{link['id']}' cannot be reverted because it has no transaction snapshot",
            code="validation",
        )
    tx = conn.execute(
        "SELECT commercial_applied_link_id FROM transactions WHERE profile_id = ? AND id = ?",
        (profile["id"], link["transaction_id"]),
    ).fetchone()
    if not tx or tx["commercial_applied_link_id"] != link["id"]:
        return False
    snapshot = json.loads(snapshot_json)
    updates = {column: snapshot.get(column) for column in TRANSACTION_APPLY_COLUMNS}
    assignments = ", ".join(f"{column} = ?" for column in updates)
    conn.execute(
        f"UPDATE transactions SET {assignments} WHERE profile_id = ? AND id = ?",
        (*updates.values(), profile["id"], link["transaction_id"]),
    )
    return True


def _btcpay_origin_attachment_label(record) -> str:
    if record["origin_kind"] == "payment_request":
        return "BTCPay payment request"
    if record["origin_kind"] == "crowdfund":
        return "BTCPay crowdfund"
    if record["origin_kind"] == "pos":
        return "BTCPay point of sale"
    return "BTCPay invoice"


def _attach_btcpay_origin_url(conn, profile, transaction_id: str, record) -> bool:
    raw_url = str(record["origin_url"] or "").strip()
    if not raw_url:
        return False
    parsed = urlparse(raw_url)
    if parsed.scheme not in {"http", "https"}:
        return False
    existing = conn.execute(
        """
        SELECT id FROM attachments
        WHERE profile_id = ?
          AND transaction_id = ?
          AND attachment_type = 'url'
          AND source_url = ?
        LIMIT 1
        """,
        (profile["id"], transaction_id, raw_url),
    ).fetchone()
    if existing:
        return False
    conn.execute(
        """
        INSERT INTO attachments(
            id, workspace_id, profile_id, transaction_id, attachment_type,
            label, source_url, media_type, created_at
        ) VALUES(?, ?, ?, ?, 'url', ?, ?, 'text/uri-list', ?)
        """,
        (
            str(uuid.uuid4()),
            profile["workspace_id"],
            profile["id"],
            transaction_id,
            _btcpay_origin_attachment_label(record),
            raw_url,
            _now(),
        ),
    )
    return True


def _apply_reviewed_link_to_transaction(conn, profile, link, commercial_kind):
    record = None
    if link["btcpay_record_id"]:
        record = conn.execute(
            "SELECT * FROM btcpay_provenance_records WHERE id = ?",
            (link["btcpay_record_id"],),
        ).fetchone()
    if not record:
        return {"applied": False, "snapshot_json": link["applied_transaction_snapshot_json"]}
    tx = conn.execute(
        f"""
        SELECT id, direction, asset, amount, {', '.join(TRANSACTION_APPLY_COLUMNS)}
        FROM transactions
        WHERE profile_id = ? AND id = ?
        """,
        (profile["id"], link["transaction_id"]),
    ).fetchone()
    if not tx:
        raise AppError(
            f"Transaction '{link['transaction_id']}' not found for commercial link '{link['id']}'",
            code="not_found",
        )
    owner = tx["commercial_applied_link_id"]
    if owner and owner != link["id"]:
        raise AppError(
            "Transaction already has reviewed BTCPay pricing from another commercial link",
            code="conflict",
            hint="Reject or revert the existing reviewed link before reviewing another one.",
            details={"transaction_id": tx["id"], "applied_link_id": owner},
        )
    existing_reviewed = conn.execute(
        """
        SELECT id FROM commercial_links
        WHERE profile_id = ?
          AND btcpay_record_id = ?
          AND link_type = 'btcpay_payment_transaction'
          AND state = 'reviewed'
          AND id != ?
        LIMIT 1
        """,
        (profile["id"], record["id"], link["id"]),
    ).fetchone()
    if existing_reviewed:
        raise AppError(
            "BTCPay payment is already reviewed against another transaction",
            code="conflict",
            hint="Reject the existing reviewed link before reviewing this payment again.",
            details={"btcpay_record_id": record["id"], "reviewed_link_id": existing_reviewed["id"]},
        )
    if record["record_type"] == "payment":
        matches = _matching_transactions_for_record(conn, profile["id"], record)
        if len(matches) != 1 or matches[0]["id"] != tx["id"]:
            raise AppError(
                "BTCPay payment matches multiple wallet transactions",
                code="ambiguous",
                hint="Resolve the duplicate txid/payment-hash rows before reviewing this provenance link.",
                details={"btcpay_record_id": record["id"], "matches": len(matches)},
            )
    if record["asset"] and record["asset"] != tx["asset"]:
        raise AppError(
            "BTCPay payment asset does not match the target transaction asset",
            code="validation",
            details={"payment_asset": record["asset"], "transaction_asset": tx["asset"]},
        )
    if record["fiat_currency"] and tx["fiat_currency"] and record["fiat_currency"] != tx["fiat_currency"]:
        raise AppError(
            "BTCPay invoice currency does not match the target transaction currency",
            code="validation",
            details={
                "payment_currency": record["fiat_currency"],
                "transaction_currency": tx["fiat_currency"],
            },
        )
    if commercial_kind == "income" and tx["direction"] != "inbound":
        raise AppError("Commercial income can only be applied to inbound transactions", code="validation")
    if commercial_kind == "expense" and tx["direction"] != "outbound":
        raise AppError("Commercial expense can only be applied to outbound transactions", code="validation")
    pair = conn.execute(
        """
        SELECT id FROM transaction_pairs
        WHERE profile_id = ?
          AND deleted_at IS NULL
          AND (out_transaction_id = ? OR in_transaction_id = ?)
        LIMIT 1
        """,
        (profile["id"], tx["id"], tx["id"]),
    ).fetchone()
    if pair:
        raise AppError(
            "Commercial review cannot overwrite a transaction that is part of an active transfer pair",
            code="validation",
            hint="Unpair the transfer first or review the merchant-side transaction instead.",
            details={"transaction_id": tx["id"], "pair_id": pair["id"]},
        )
    updates: dict[str, Any] = {}
    snapshot_json = (
        link["applied_transaction_snapshot_json"]
        if tx["commercial_applied_link_id"] == link["id"] and link["applied_transaction_snapshot_json"]
        else _transaction_snapshot(tx)
    )
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
        if record["fiat_currency"]:
            updates["fiat_currency"] = record["fiat_currency"]
    if commercial_kind and commercial_kind != "transfer":
        updates["kind"] = commercial_kind
    elif commercial_kind is None or commercial_kind == "transfer":
        updates["kind"] = json.loads(snapshot_json).get("kind")
    if not updates:
        return {"applied": False, "snapshot_json": snapshot_json}
    updates["commercial_applied_link_id"] = link["id"]
    assignments = ", ".join(f"{column} = ?" for column in updates)
    conn.execute(
        f"UPDATE transactions SET {assignments} WHERE profile_id = ? AND id = ?",
        (*updates.values(), profile["id"], link["transaction_id"]),
    )
    _attach_btcpay_origin_url(conn, profile, link["transaction_id"], record)
    return {"applied": True, "snapshot_json": snapshot_json}


def build_reviewed_subledger_rows(conn, workspace_ref, profile_ref, hooks: CommercialHooks):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT cl.id AS link_id, cl.commercial_kind, cl.allocation_amount,
               cl.allocation_fiat_exact, cl.reconciliation_state,
               t.id AS transaction_id, t.external_id, t.occurred_at, t.direction,
               t.asset, t.amount, t.fee, t.fiat_currency, t.fiat_value_exact,
               t.pricing_source_kind, w.label AS wallet,
               p.invoice_id, p.payment_id, p.order_id, p.payment_request_id,
               p.origin_kind, p.origin_label, p.stable_key,
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
        "payment_request_id",
        "origin_kind",
        "origin_label",
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
        "payment_request_id": row["payment_request_id"] or "",
        "origin_kind": row["origin_kind"] or "",
        "origin_app_id": row["origin_app_id"] or "",
        "origin_label": row["origin_label"] or "",
        "origin_url": row["origin_url"] or "",
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value_exact": row["fiat_value_exact"] or "",
        "fiat_rate_exact": row["fiat_rate_exact"] or "",
        "pricing_timestamp": row["pricing_timestamp"] or "",
        "updated_at": row["updated_at"],
    }


def _document_context_payload(row):
    return {
        "id": row["ctx_document_id"],
        "document_type": row["ctx_document_type"],
        "label": row["ctx_document_label"],
        "external_ref": row["ctx_document_external_ref"] or "",
        "review_state": row["ctx_document_review_state"],
    }


def _btcpay_record_context_payload(row, prefix):
    record_id = row[f"{prefix}_record_id"]
    if not record_id:
        return None
    amount_msat = row[f"{prefix}_amount"]
    return {
        "id": record_id,
        "record_type": row[f"{prefix}_record_type"],
        "invoice_id": row[f"{prefix}_invoice_id"] or "",
        "payment_id": row[f"{prefix}_payment_id"] or "",
        "order_id": row[f"{prefix}_order_id"] or "",
        "status": row[f"{prefix}_status"] or "",
        "occurred_at": row[f"{prefix}_occurred_at"] or "",
        "asset": row[f"{prefix}_asset"] or "",
        "amount_msat": amount_msat,
        "amount": float(msat_to_btc(amount_msat)) if amount_msat is not None else None,
        "payment_request_id": row[f"{prefix}_payment_request_id"] or "",
        "origin_kind": row[f"{prefix}_origin_kind"] or "",
        "origin_app_id": row[f"{prefix}_origin_app_id"] or "",
        "origin_label": row[f"{prefix}_origin_label"] or "",
        "origin_url": row[f"{prefix}_origin_url"] or "",
        "fiat_currency": row[f"{prefix}_fiat_currency"] or "",
        "fiat_value_exact": row[f"{prefix}_fiat_value_exact"] or "",
        "fiat_rate_exact": row[f"{prefix}_fiat_rate_exact"] or "",
        "pricing_timestamp": row[f"{prefix}_pricing_timestamp"] or "",
        "updated_at": row[f"{prefix}_updated_at"],
    }


def _payment_request_context(payment, invoice):
    source = invoice or payment
    if not source or not source.get("payment_request_id"):
        return None
    return {
        "id": source["payment_request_id"],
        "label": source.get("origin_label") or source["payment_request_id"],
        "status": source.get("status") or "",
        "url": source.get("origin_url") or "",
    }


def _origin_context(payment, invoice):
    source = invoice or payment
    if not source:
        return None
    kind = source.get("origin_kind") or ""
    if not kind or kind == "unknown":
        return None
    return {
        "kind": kind,
        "app_id": source.get("origin_app_id") or "",
        "label": source.get("origin_label") or "",
        "url": source.get("origin_url") or "",
    }


def _link_context_payload(row):
    return {
        "id": row["id"],
        "invoice_id": row["invoice_id"] or "",
        "payment_id": row["payment_id"] or "",
        "document_id": row["document_id"] or "",
        "document_label": row["document_label"] or "",
        "link_type": row["link_type"],
        "state": row["state"],
        "confidence": row["confidence"],
        "reconciliation_state": row["reconciliation_state"],
        "commercial_kind": row["commercial_kind"] or "",
        "reviewed_at": row["reviewed_at"] or "",
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
        "payment_request_id": row["payment_request_id"] or "",
        "origin_kind": row["origin_kind"] or "",
        "origin_label": row["origin_label"] or "",
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
        "reviewed_record_snapshot_sha256": row["reviewed_record_snapshot_sha256"] or "",
        "has_applied_transaction_snapshot": bool(row["applied_transaction_snapshot_json"]),
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
        "payment_request_id": row["payment_request_id"] or "",
        "origin_kind": row["origin_kind"] or "",
        "origin_label": row["origin_label"] or "",
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
    "get_transaction_commercial_context",
    "list_btcpay_records",
    "list_documents",
    "list_links",
    "review_link",
    "suggest_links",
    "upsert_btcpay_provenance",
]
