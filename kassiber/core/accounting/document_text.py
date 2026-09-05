"""Explicit local extraction and reviewed fields for encrypted accounting evidence.

No provider calls, URLs, model instructions or accounting postings live here.
Text and corrections are immutable SQLCipher records; ordinary evidence lists
continue to expose metadata only.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import uuid
from datetime import date

from kassiber.errors import AppError
from . import evidence, ledger

MAX_TEXT_BYTES = 2 * 1024 * 1024
MAX_PAGES = 2000
FIELDS = frozenset({"document_type", "document_number", "issued_date", "due_date",
                    "currency", "minor_unit_exponent", "counterparty", "net_minor", "vat_minor", "total_minor"})


def ensure_schema(conn):
    conn.execute("""CREATE TABLE IF NOT EXISTS gl_evidence_extractions (
        id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, evidence_id TEXT NOT NULL,
        source_digest TEXT NOT NULL, method TEXT NOT NULL, tool_version TEXT NOT NULL,
        pages_json TEXT NOT NULL, content_digest TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        UNIQUE(profile_id,id),
        FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id) ON DELETE RESTRICT)""")
    conn.execute("""CREATE TABLE IF NOT EXISTS gl_evidence_field_reviews (
        id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, extraction_id TEXT NOT NULL,
        previous_id TEXT, fields_json TEXT NOT NULL, spans_json TEXT NOT NULL,
        reason TEXT NOT NULL, content_digest TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        UNIQUE(profile_id,id), UNIQUE(extraction_id,previous_id),
        FOREIGN KEY(profile_id,extraction_id) REFERENCES gl_evidence_extractions(profile_id,id),
        FOREIGN KEY(profile_id,previous_id) REFERENCES gl_evidence_field_reviews(profile_id,id))""")
    conn.execute("""CREATE UNIQUE INDEX IF NOT EXISTS gl_evidence_first_field_review
        ON gl_evidence_field_reviews(extraction_id) WHERE previous_id IS NULL""")
    for table in ("gl_evidence_extractions", "gl_evidence_field_reviews"):
        for operation in ("UPDATE", "DELETE"):
            conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_{operation.lower()}
                BEFORE {operation} ON {table} BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS {table}_no_replace
            BEFORE INSERT ON {table} WHEN EXISTS(SELECT 1 FROM {table} WHERE id=NEW.id)
            BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")


def capabilities():
    isolated = os.name == "posix"
    return {"utf8_text": True, "pdf_text": isolated and shutil.which("pdftotext") is not None,
            "parser_isolation_supported": isolated,
            "ocr_images": isolated and shutil.which("tesseract") is not None,
            "ocr_pdf": isolated and shutil.which("tesseract") is not None and shutil.which("pdftoppm") is not None,
            "ocr_max_pages": 8, "ocr_image_types": ["image/png", "image/jpeg"],
            "network_required": False, "max_text_bytes": MAX_TEXT_BYTES}


def _worker_command(kind: str, arguments: list[str]) -> list[str]:
    if kind not in {"text", "ocr"}:
        raise AppError("Unknown document worker", code="accounting_invalid_fields")
    if getattr(sys, "frozen", False):
        return [sys.executable, "--accounting-document-worker", kind, *arguments]
    name = "_document_text_worker.py" if kind == "text" else "_document_ocr_worker.py"
    return [sys.executable, "-I", str(Path(__file__).with_name(name)), *arguments]


def _pdf_text(content: bytes, cancel: threading.Event | None = None) -> tuple[list[str], str]:
    executable = shutil.which("pdftotext")
    if executable is None:
        raise AppError("Local PDF text tool is not installed; use reviewed manual transcription",
                       code="accounting_pdf_text_unavailable")
    # Isolated trusted worker has no book path/key or provider credentials. No
    # plaintext source/output files are created. Kill the entire parser group
    # on timeout so its child cannot survive an abandoned extraction.
    return _run_worker("text", [executable], content, cancel, timeout=30)


def _run_worker(kind, arguments, content, cancel, *, timeout):
    if cancel is not None and cancel.is_set():
        raise AppError("Document extraction cancelled", code="accounting_document_cancelled")
    # A parent-only kill does not terminate native parser descendants on
    # Windows. Until a tested job-object boundary exists, fail before giving
    # the parser any bytes; plain UTF-8 and reviewed transcription still work.
    if os.name != "posix":
        raise AppError("Isolated PDF/OCR subprocesses are unavailable on this platform; use manual transcription",
                       code="accounting_document_isolation_unavailable")
    environment = {"PATH": os.defpath, "LANG": "C", "LC_ALL": "C"}
    try:
        process = subprocess.Popen(_worker_command(kind, arguments),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=environment, start_new_session=os.name == "posix")
    except OSError:
        raise AppError("Local document extraction could not start", code="accounting_document_parse_failed") from None
    try:
        deadline = time.monotonic() + timeout
        first = True
        while True:
            if cancel is not None and cancel.is_set():
                raise AppError("Document extraction cancelled", code="accounting_document_cancelled")
            if time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(process.args, timeout)
            try:
                output, _ = process.communicate(content if first else None, timeout=0.05)
                break
            except subprocess.TimeoutExpired:
                first = False
    except BaseException as exc:
        try:
            if os.name == "posix":
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
        except ProcessLookupError:
            pass
        process.communicate()
        if isinstance(exc, subprocess.TimeoutExpired):
            raise AppError("Local document extraction exceeded its resource budget",
                           code="accounting_document_parse_failed") from None
        raise
    if process.returncode or len(output) > 12 * MAX_TEXT_BYTES:
        raise AppError("Local document extraction failed; use reviewed manual transcription",
                       code="accounting_document_parse_failed")
    try:
        result = json.loads(output)
        if not isinstance(result, dict):
            raise ValueError()
        if result.get("error") == "accounting_ocr_language_unavailable":
            raise AppError("Requested OCR language is not installed; choose an installed language or use manual transcription",
                           code="accounting_ocr_language_unavailable")
        if not isinstance(result.get("version"), str) or len(result["version"]) > 2000:
            raise ValueError()
        return result["pages"], result["version"]
    except (ValueError, KeyError, TypeError):
        raise AppError("Local document extraction failed", code="accounting_document_parse_failed") from None


def _ocr_text(content, media_type, pages, language, cancel):
    if media_type not in {"application/pdf", "image/png", "image/jpeg"}:
        raise AppError("Local OCR supports PNG, JPEG and selected PDF pages", code="accounting_document_text_unsupported")
    if pages is None and media_type != "application/pdf":
        pages = [1]
    if not isinstance(pages, list) or not 1 <= len(pages) <= 8 or any(type(p) is not int or not 1 <= p <= 2000 for p in pages) or len(set(pages)) != len(pages) or (media_type != "application/pdf" and pages != [1]):
        raise AppError("Select one to eight explicit PDF page numbers; image OCR uses page one", code="accounting_ocr_pages_invalid")
    language = "eng" if language is None else language
    if not isinstance(language, str) or not re.fullmatch(r"[A-Za-z0-9_]{1,32}(?:\+[A-Za-z0-9_]{1,32}){0,2}", language):
        raise AppError("Invalid local OCR language", code="accounting_ocr_language_invalid")
    executable = shutil.which("tesseract")
    if executable is None:
        raise AppError("Optional local Tesseract is not installed; install it explicitly or use manual transcription",
                       code="accounting_ocr_unavailable")
    renderer = shutil.which("pdftoppm") if media_type == "application/pdf" else ""
    if renderer is None:
        raise AppError("Selected-page OCR needs optional local Poppler; use manual transcription",
                       code="accounting_ocr_pdf_renderer_unavailable")
    return _run_worker("ocr", [executable, renderer, media_type, language, json.dumps(pages)], content, cancel, timeout=120)


def _pages(pages):
    if not isinstance(pages, list) or not 1 <= len(pages) <= MAX_PAGES:
        raise AppError("Invalid extraction pages", code="accounting_document_text_invalid")
    if any(not isinstance(page, str) or "\x00" in page for page in pages):
        raise AppError("Invalid extraction text", code="accounting_document_text_invalid")
    try:
        size = sum(len(page.encode("utf-8")) for page in pages)
    except UnicodeError:
        raise AppError("Extraction text must be valid Unicode", code="accounting_document_text_invalid") from None
    if size > MAX_TEXT_BYTES or not any(page.strip() for page in pages):
        raise AppError("No usable bounded text; use local OCR or manual transcription",
                       code="accounting_document_text_empty_or_large")
    return pages


def extract(conn, profile_id: str, *, evidence_id: str, method="text", ocr_pages=None, ocr_language=None):
    ledger.require_book(conn, profile_id)
    source = evidence.require_evidence(conn, profile_id, evidence_id)
    content = evidence.read_evidence_bytes(conn, profile_id, evidence_id)
    pages, method, version = extract_bytes(content, source["media_type"], method=method, ocr_pages=ocr_pages, ocr_language=ocr_language)
    return _retain(conn, profile_id, source, pages, method, version)


def extract_bytes(content: bytes, media_type: str, *, cancel: threading.Event | None = None,
                  method="text", ocr_pages=None, ocr_language=None):
    """DB-free worker boundary; never pass a connection, key, or file path."""
    if cancel is not None and cancel.is_set():
        raise AppError("Document extraction cancelled", code="accounting_document_cancelled")
    if not isinstance(method, str) or method not in {"text", "ocr"} or (method == "text" and (ocr_pages is not None or ocr_language is not None)):
        raise AppError("Invalid document extraction method or options", code="accounting_invalid_fields")
    if method == "ocr":
        pages, version = _ocr_text(content, media_type, ocr_pages, ocr_language, cancel)
        method = "tesseract"
    elif media_type in {"text/plain", "text/csv"}:
        try:
            pages = [content.decode("utf-8-sig")]
        except UnicodeError:
            raise AppError("Text evidence must be UTF-8", code="accounting_document_text_invalid") from None
        method, version = "utf8", "1"
    elif media_type == "application/pdf":
        pages, version = _pdf_text(content, cancel)
        method = "pdftotext"
    else:
        raise AppError("This evidence needs local OCR or manual transcription",
                       code="accounting_document_text_unsupported")
    return _pages(pages), method, version


def transcribe(conn, profile_id: str, *, evidence_id: str, pages: list[str], reason: str):
    ledger.require_book(conn, profile_id)
    evidence.bounded_text(reason, "reason", 2000)
    source = evidence.require_evidence(conn, profile_id, evidence_id)
    # Manual transcription is visibly distinguished from verified parser output.
    return _retain(conn, profile_id, source, _pages(pages), "manual", reason)


def _retain(conn, profile_id, source, pages, method, version):
    with ledger.atomic(conn):
        ledger.require_book(conn, profile_id)
        current = evidence.require_evidence(conn, profile_id, source["id"])
        if current["content_sha256"] != source["content_sha256"]:
            raise AppError("Evidence changed", code="accounting_stale_approval")
        identifier = str(uuid.uuid4())
        payload = {"source_digest": source["content_sha256"], "method": method,
                   "tool_version": version, "pages": pages}
        conn.execute("""INSERT INTO gl_evidence_extractions
            (id,profile_id,evidence_id,source_digest,method,tool_version,pages_json,content_digest)
            VALUES (?,?,?,?,?,?,?,?)""", (identifier, profile_id, source["id"],
            source["content_sha256"], method, version, ledger.canonical_json(pages), ledger.digest(payload)))
        ledger._bump(conn, profile_id)
        return get(conn, profile_id, extraction_id=identifier)


def get(conn, profile_id: str, *, extraction_id: str):
    ledger.require_book(conn, profile_id)
    row = conn.execute("SELECT * FROM gl_evidence_extractions WHERE profile_id=? AND id=?",
                       (profile_id, extraction_id)).fetchone()
    if row is None:
        raise AppError("Extraction not found in this book", code="accounting_extraction_not_found")
    result = dict(row)
    result["pages"] = json.loads(result.pop("pages_json"))
    payload = {key: result[key] for key in ("source_digest", "method", "tool_version", "pages")}
    if ledger.digest(payload) != result["content_digest"]:
        raise AppError("Extraction digest mismatch", code="accounting_evidence_corrupt")
    review = conn.execute("""SELECT * FROM gl_evidence_field_reviews r WHERE r.profile_id=?
        AND r.extraction_id=? AND NOT EXISTS(SELECT 1 FROM gl_evidence_field_reviews next
            WHERE next.previous_id=r.id)""", (profile_id, extraction_id)).fetchone()
    result["review"] = None
    if review:
        value = dict(review)
        value["fields"] = json.loads(value.pop("fields_json"))
        value["spans"] = json.loads(value.pop("spans_json"))
        review_payload = {key: value[key] for key in ("extraction_id", "previous_id", "fields", "spans", "reason")}
        if ledger.digest(review_payload) != value["content_digest"]:
            raise AppError("Document review digest mismatch", code="accounting_evidence_corrupt")
        result["review"] = value
    return result


def review_fields(conn, profile_id: str, *, extraction_id: str, expected_digest: str,
                  previous_id: str | None, fields: dict, spans: dict, reason: str):
    evidence.bounded_text(reason, "reason", 2000)
    if not isinstance(fields, dict) or not fields or set(fields) - FIELDS:
        raise AppError("Invalid document fields", code="accounting_document_fields_invalid")
    if not isinstance(spans, dict) or set(spans) - set(fields):
        raise AppError("Invalid document source spans", code="accounting_document_fields_invalid")
    for key, value in fields.items():
        if key == "minor_unit_exponent":
            if type(value) is not int or not 0 <= value <= 8:
                raise AppError("Invalid document currency exponent", code="accounting_document_fields_invalid")
        elif key.endswith("_minor"):
            if type(value) is not int or abs(value) > 2**63 - 1:
                raise AppError("Document amounts require exact integer minor units", code="accounting_invalid_amount")
        else:
            evidence.bounded_text(value, key, 500)
        if key in {"issued_date", "due_date"}:
            try:
                if date.fromisoformat(value).isoformat() != value:
                    raise ValueError()
            except ValueError:
                raise AppError("Invalid document date", code="accounting_document_fields_invalid") from None
        if key == "currency" and not re.fullmatch("[A-Z]{3}", value):
            raise AppError("Invalid currency", code="accounting_document_fields_invalid")
        if key == "document_type" and value not in {"invoice", "receipt", "credit_note", "statement", "other"}:
            raise AppError("Invalid document type", code="accounting_document_fields_invalid")
    if {"net_minor", "vat_minor", "total_minor"} <= fields.keys():
        if fields["net_minor"] + fields["vat_minor"] != fields["total_minor"]:
            raise AppError("Document totals do not reconcile", code="accounting_document_totals_mismatch")
    if any(key.endswith("_minor") for key in fields) and not {"currency", "minor_unit_exponent"} <= fields.keys():
        raise AppError("Reviewed amounts require explicit source currency and exponent", code="accounting_document_fields_invalid")
    with ledger.atomic(conn):
        record = get(conn, profile_id, extraction_id=extraction_id)
        if record["content_digest"] != expected_digest or (record["review"] or {}).get("id") != previous_id:
            raise AppError("Document review changed", code="accounting_stale_approval")
        for span in spans.values():
            if not isinstance(span, dict) or set(span) != {"page", "start", "end"} or any(type(x) is not int for x in span.values()):
                raise AppError("Invalid source span", code="accounting_document_fields_invalid")
            page, start, end = span["page"], span["start"], span["end"]
            if not 1 <= page <= len(record["pages"]) or not 0 <= start < end <= len(record["pages"][page - 1]):
                raise AppError("Source span outside extracted text", code="accounting_document_fields_invalid")
        identifier = str(uuid.uuid4())
        payload = {"extraction_id": extraction_id, "previous_id": previous_id,
                   "fields": fields, "spans": spans, "reason": reason}
        conn.execute("""INSERT INTO gl_evidence_field_reviews
            (id,profile_id,extraction_id,previous_id,fields_json,spans_json,reason,content_digest)
            VALUES (?,?,?,?,?,?,?,?)""", (identifier, profile_id, extraction_id, previous_id,
            ledger.canonical_json(fields), ledger.canonical_json(spans), reason, ledger.digest(payload)))
        ledger._bump(conn, profile_id)
        return get(conn, profile_id, extraction_id=extraction_id)


def search(conn, profile_id: str, *, query: str, limit: int = 50):
    ledger.require_book(conn, profile_id)
    evidence.bounded_text(query, "query", 200)
    if type(limit) is not int or not 1 <= limit <= 100:
        raise AppError("Invalid search limit", code="accounting_invalid_input")
    # Literal instr rather than FTS query syntax or wildcard interpolation. The
    # search index and snippets never leave the encrypted database by default.
    rows = conn.execute("""SELECT id,evidence_id,method,created_at FROM gl_evidence_extractions
        WHERE profile_id=? AND instr(lower(pages_json),lower(?))>0 ORDER BY created_at DESC,id DESC LIMIT ?""",
        (profile_id, query, limit + 1)).fetchall()
    return {"extractions": [dict(row) for row in rows[:limit]], "has_more": len(rows) > limit}
