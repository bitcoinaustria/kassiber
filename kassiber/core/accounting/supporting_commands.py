"""Explicit supporting-record operations; never arbitrary document or SQL access."""

from __future__ import annotations

import base64
import binascii

from ...errors import AppError

READ_ACTIONS = frozenset({"evidence-list", "evidence-upload-list", "bank-preview", "bank-list", "bank-reconcile", "item-list", "schedule-list"})
WRITE_ACTIONS = frozenset({
    "evidence-add", "bank-import", "bank-allocate", "bank-void-allocation", "bank-void-statement",
    "evidence-upload-begin", "evidence-upload-append", "evidence-upload-finish", "evidence-upload-cancel",
    "item-create", "item-revise", "item-allocate", "item-void", "item-void-settlement", "schedule-create", "schedule-revise",
})


def execute_supporting(conn, profile_id, action, payload):
    from . import bank, evidence, schedules
    from .commands import _fields
    from .ledger import require_book

    require_book(conn, profile_id)
    p = payload

    def bank_input(required, extra=()):
        args = _fields(p, {"csv_text", "csv_evidence_id", "start_date", "end_date", "opening_minor", "closing_minor", *extra}, required)
        if ("csv_text" in args) == ("csv_evidence_id" in args):
            raise AppError("Choose exactly one CSV input source", code="accounting_invalid_fields")
        if "csv_evidence_id" in args:
            source_id = args.pop("csv_evidence_id")
            if action == "bank-import":
                # The canonical retained CSV is this adapter's stable source.
                # Do not discard its identity in favor of an unrelated PDF.
                if args.get("evidence_id") not in (None, source_id):
                    raise AppError("Retained CSV imports must keep their source evidence identity", code="accounting_evidence_scope")
                args["evidence_id"] = source_id
            raw = evidence.read_evidence_bytes(conn, profile_id, source_id)
            if len(raw) > 4 * 1024 * 1024:
                raise AppError("Bank CSV exceeds its size limit", code="accounting_invalid_input")
            try:
                args["csv_text"] = raw.decode("utf-8-sig")
            except UnicodeError as exc:
                raise AppError("Retained bank CSV must be UTF-8", code="accounting_invalid_input") from exc
        return args
    if action == "evidence-list":
        return evidence.evidence_page(conn, profile_id, **_fields(p, {"limit", "cursor"}))
    if action == "evidence-upload-list":
        _fields(p, set())
        return {"uploads": evidence.list_uploads(conn, profile_id)}
    if action == "evidence-upload-begin":
        args = _fields(p, {"name", "media_type", "total_bytes", "content_sha256", "idempotency_key", "source_document_id"}, {"name", "media_type", "total_bytes", "content_sha256", "idempotency_key"})
        return evidence.begin_upload(conn, profile_id, **args)
    if action == "evidence-upload-append":
        args = _fields(p, {"upload_id", "offset", "content_base64", "chunk_sha256"}, {"upload_id", "offset", "content_base64", "chunk_sha256"})
        encoded = args.pop("content_base64")
        if not isinstance(encoded, str) or len(encoded) > 4 * ((256 * 1024 + 2) // 3):
            raise AppError("Evidence chunk exceeds its size limit", code="accounting_evidence_size")
        try:
            content = base64.b64decode(encoded, validate=True)
        except (ValueError, binascii.Error) as exc:
            raise AppError("Evidence chunk must be valid base64", code="accounting_invalid_input") from exc
        return evidence.append_upload(conn, profile_id, content=content, **args)
    if action == "evidence-upload-finish":
        return evidence.finish_upload(conn, profile_id, **_fields(p, {"upload_id"}, {"upload_id"}))
    if action == "evidence-upload-cancel":
        return evidence.cancel_upload(conn, profile_id, **_fields(p, {"upload_id"}, {"upload_id"}))
    if action == "evidence-add":
        args = _fields(p, {"content_base64", "name", "media_type", "source_document_id"}, {"content_base64", "name", "media_type"})
        encoded = args.pop("content_base64")
        if not isinstance(encoded, str) or len(encoded) > 4 * ((evidence.MAX_EVIDENCE_BYTES + 2) // 3):
            raise AppError("Evidence payload exceeds its limit", code="accounting_evidence_size")
        try:
            content = base64.b64decode(encoded, validate=True)
        except (ValueError, binascii.Error) as exc:
            raise AppError("Evidence must be valid base64", code="accounting_invalid_input") from exc
        return evidence.retain_evidence(conn, profile_id, content=content, **args)
    if action == "bank-preview":
        args = bank_input({"start_date", "end_date"})
        return bank.preview_statement(**args)
    if action == "bank-list":
        return bank.statements_page(conn, profile_id, **_fields(p, {"limit", "cursor"}))
    if action == "bank-import":
        args = bank_input({"start_date", "end_date", "account_code", "statement_id"},
                         {"account_code", "statement_id", "evidence_id", "control_evidence_id", "control_review_reason", "control_locator"})
        return bank.import_statement(conn, profile_id, **args)
    if action == "bank-allocate":
        args = _fields(p, {"row_id", "line_id", "amount_minor", "idempotency_key"}, {"row_id", "line_id", "amount_minor", "idempotency_key"})
        return bank.allocate_bank_row(conn, profile_id, **args)
    if action == "bank-reconcile":
        args = _fields(p, {"statement_id"}, {"statement_id"})
        return bank.reconcile_statement(conn, profile_id, **args)
    if action == "bank-void-allocation":
        args = _fields(p, {"allocation_id", "reason", "idempotency_key"}, {"allocation_id", "reason", "idempotency_key"})
        return bank.void_bank_allocation(conn, profile_id, **args)
    if action == "bank-void-statement":
        args = _fields(p, {"statement_id", "reason", "idempotency_key"}, {"statement_id", "reason", "idempotency_key"})
        return bank.void_statement(conn, profile_id, **args)
    if action == "item-list":
        return schedules.items_page(conn, profile_id, **_fields(p, {"limit", "cursor"}))
    if action == "item-create":
        args = _fields(p, {"direction", "document_ref", "origin_line_id", "evidence_id", "due_date"}, {"direction", "document_ref", "origin_line_id", "evidence_id", "due_date"})
        return schedules.create_open_item(conn, profile_id, **args)
    if action == "item-allocate":
        args = _fields(p, {"item_id", "settlement_line_id", "amount_minor", "idempotency_key"}, {"item_id", "settlement_line_id", "amount_minor", "idempotency_key"})
        return schedules.allocate_settlement(conn, profile_id, **args)
    if action == "item-revise":
        fields = {"item_id", "expected_revision", "expected_digest", "document_ref", "due_date", "effective_date", "evidence_id", "reason", "idempotency_key"}
        return schedules.revise_open_item(conn, profile_id, **_fields(p, fields, fields))
    if action == "item-void":
        args = _fields(p, {"item_id", "reason", "idempotency_key"}, {"item_id", "reason", "idempotency_key"})
        return schedules.void_open_item(conn, profile_id, **args)
    if action == "item-void-settlement":
        args = _fields(p, {"allocation_id", "reason", "idempotency_key"}, {"allocation_id", "reason", "idempotency_key"})
        return schedules.void_settlement(conn, profile_id, **args)
    if action == "schedule-list":
        return schedules.schedules_page(conn, profile_id, **_fields(p, {"limit", "cursor"}))
    if action == "schedule-create":
        args = _fields(p, {"kind", "label", "effective_date", "evidence_id", "fields", "reason", "entry_id"}, {"kind", "label", "effective_date", "evidence_id", "fields", "reason"})
        return schedules.create_schedule(conn, profile_id, **args)
    if action == "schedule-revise":
        args = _fields(p, {"schedule_id", "expected_revision", "effective_date", "evidence_id", "fields", "reason", "entry_id"}, {"schedule_id", "expected_revision", "effective_date", "evidence_id", "fields", "reason"})
        return schedules.revise_schedule(conn, profile_id, **args)
    raise AppError("Unknown supporting-record operation", code="accounting_unknown_operation")
