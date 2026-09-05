"""Bounded immutable accounting evidence stored inside the encrypted book.

No path, URL, parser, or network API lives here. Callers supply reviewed bytes.
Metadata reads deliberately omit document content and extracted text.
"""
from __future__ import annotations

import hashlib
import re
import uuid
from typing import Any

from kassiber.errors import AppError
from .ledger import atomic, require_book

MAX_EVIDENCE_BYTES = 20 * 1024 * 1024
MAX_UPLOAD_CHUNK_BYTES = 256 * 1024


def ensure_schema(conn: Any) -> None:
    conn.execute("""CREATE TABLE IF NOT EXISTS gl_evidence (
        id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
        name TEXT NOT NULL, media_type TEXT NOT NULL, content BLOB NOT NULL,
        content_sha256 TEXT NOT NULL, byte_length INTEGER NOT NULL,
        source_document_id TEXT REFERENCES external_documents(id) ON DELETE RESTRICT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        UNIQUE(profile_id,id), CHECK(byte_length > 0 AND byte_length <= 20971520)
    )""")
    for action in ("UPDATE", "DELETE"):
        conn.execute(f"""CREATE TRIGGER IF NOT EXISTS gl_evidence_no_{action.lower()}
            BEFORE {action} ON gl_evidence BEGIN
            SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_no_replace
        BEFORE INSERT ON gl_evidence WHEN EXISTS (SELECT 1 FROM gl_evidence WHERE id=NEW.id)
        BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_document_scope
        BEFORE INSERT ON gl_evidence WHEN NEW.source_document_id IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM external_documents WHERE id=NEW.source_document_id
            AND profile_id=NEW.profile_id)
        BEGIN SELECT RAISE(ABORT,'accounting_evidence_scope'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_document_retained
        BEFORE DELETE ON external_documents WHEN EXISTS
        (SELECT 1 FROM gl_evidence WHERE source_document_id=OLD.id)
        BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_document_scope_retained
        BEFORE UPDATE ON external_documents WHEN (NEW.id!=OLD.id OR NEW.profile_id!=OLD.profile_id)
        AND EXISTS (SELECT 1 FROM gl_evidence WHERE source_document_id=OLD.id)
        BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_document_no_replace
        BEFORE INSERT ON external_documents WHEN EXISTS
        (SELECT 1 FROM gl_evidence WHERE source_document_id=NEW.id)
        BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
    conn.execute("""CREATE TABLE IF NOT EXISTS gl_evidence_uploads (
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT,
        name TEXT NOT NULL,media_type TEXT NOT NULL,total_bytes INTEGER NOT NULL,
        content_sha256 TEXT NOT NULL,idempotency_key TEXT NOT NULL,
        source_document_id TEXT REFERENCES external_documents(id) ON DELETE RESTRICT,
        evidence_id TEXT REFERENCES gl_evidence(id),UNIQUE(profile_id,idempotency_key),UNIQUE(profile_id,id),
        CHECK(typeof(total_bytes)='integer' AND total_bytes>0 AND total_bytes<=20971520))""")
    conn.execute("""CREATE TABLE IF NOT EXISTS gl_evidence_upload_chunks (
        upload_id TEXT NOT NULL,profile_id TEXT NOT NULL,offset INTEGER NOT NULL,content BLOB NOT NULL,
        chunk_sha256 TEXT NOT NULL,PRIMARY KEY(upload_id,offset),
        CHECK(typeof(offset)='integer' AND offset>=0),CHECK(length(content)>0 AND length(content)<=262144),
        FOREIGN KEY(profile_id,upload_id) REFERENCES gl_evidence_uploads(profile_id,id) ON DELETE CASCADE)""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_upload_scope BEFORE INSERT ON gl_evidence_uploads
        WHEN NEW.source_document_id IS NOT NULL AND NOT EXISTS
        (SELECT 1 FROM external_documents WHERE id=NEW.source_document_id AND profile_id=NEW.profile_id)
        BEGIN SELECT RAISE(ABORT,'accounting_evidence_scope'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_upload_no_replace BEFORE INSERT ON gl_evidence_uploads
        WHEN EXISTS (SELECT 1 FROM gl_evidence_uploads WHERE id=NEW.id OR
          (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key))
        BEGIN SELECT RAISE(ABORT,'accounting_upload_immutable'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_upload_no_rewrite BEFORE UPDATE ON gl_evidence_uploads
        WHEN OLD.evidence_id IS NOT NULL OR NEW.id!=OLD.id OR NEW.profile_id!=OLD.profile_id
          OR NEW.name!=OLD.name OR NEW.media_type!=OLD.media_type OR NEW.total_bytes!=OLD.total_bytes
          OR NEW.content_sha256!=OLD.content_sha256 OR NEW.idempotency_key!=OLD.idempotency_key
          OR NEW.source_document_id IS NOT OLD.source_document_id
          OR NOT EXISTS (SELECT 1 FROM gl_evidence WHERE id=NEW.evidence_id AND profile_id=OLD.profile_id
             AND content_sha256=OLD.content_sha256 AND byte_length=OLD.total_bytes)
        BEGIN SELECT RAISE(ABORT,'accounting_upload_immutable'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_upload_retained BEFORE DELETE ON gl_evidence_uploads
        WHEN OLD.evidence_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_chunk_no_replace BEFORE INSERT ON gl_evidence_upload_chunks
        WHEN EXISTS (SELECT 1 FROM gl_evidence_upload_chunks WHERE upload_id=NEW.upload_id AND offset=NEW.offset)
        BEGIN SELECT RAISE(ABORT,'accounting_upload_immutable'); END""")
    conn.execute("""CREATE TRIGGER IF NOT EXISTS gl_evidence_chunk_no_update BEFORE UPDATE ON gl_evidence_upload_chunks
        BEGIN SELECT RAISE(ABORT,'accounting_upload_immutable'); END""")


def bounded_text(value: Any, field: str, maximum: int = 256) -> str:
    if not isinstance(value, str) or not value.strip() or len(value) > maximum or "\x00" in value:
        raise AppError(f"Invalid {field}", code="accounting_invalid_input")
    return value


def require_evidence(conn: Any, profile_id: str, evidence_id: str) -> dict:
    row = conn.execute("""SELECT id,profile_id,name,media_type,content_sha256,byte_length,
        source_document_id,created_at FROM gl_evidence WHERE profile_id=? AND id=?""",
        (profile_id, evidence_id)).fetchone()
    if row is None:
        raise AppError("Evidence not found in this book", code="accounting_evidence_not_found")
    return dict(row)


def retain_evidence(conn: Any, profile_id: str, *, content: bytes, media_type: str,
                    name: str, source_document_id: str | None = None) -> dict:
    with atomic(conn):
        require_book(conn, profile_id)
        bounded_text(name, "name")
        bounded_text(media_type, "media_type", 128)
        if not isinstance(content, bytes) or not 0 < len(content) <= MAX_EVIDENCE_BYTES:
            raise AppError("Evidence must contain at most 20 MiB", code="accounting_evidence_size")
        if source_document_id is not None:
            row = conn.execute("SELECT 1 FROM external_documents WHERE id=? AND profile_id=?",
                               (source_document_id, profile_id)).fetchone()
            if row is None:
                raise AppError("Document not found in this book", code="accounting_evidence_scope")
        evidence_id = str(uuid.uuid4())
        conn.execute("""INSERT INTO gl_evidence
            (id,profile_id,name,media_type,content,content_sha256,byte_length,source_document_id)
            VALUES (?,?,?,?,?,?,?,?)""", (evidence_id, profile_id, name, media_type, content,
            hashlib.sha256(content).hexdigest(), len(content), source_document_id))
        return require_evidence(conn, profile_id, evidence_id)


def list_evidence(conn: Any, profile_id: str, *, limit: int = 100) -> list[dict]:
    require_book(conn, profile_id)
    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError("Invalid evidence limit", code="accounting_invalid_input")
    return [dict(row) for row in conn.execute("""SELECT id,name,media_type,content_sha256,
        byte_length,source_document_id,created_at FROM gl_evidence WHERE profile_id=?
        ORDER BY created_at,id LIMIT ?""", (profile_id, limit))]


def evidence_page(conn: Any, profile_id: str, *, limit: int = 100, cursor: str | None = None) -> dict:
    from .paging import records_page
    return records_page(conn, profile_id, "evidence", limit=limit, cursor=cursor,
                        materialize=lambda identifier: require_evidence(conn, profile_id, identifier))


def read_evidence_bytes(conn: Any, profile_id: str, evidence_id: str) -> bytes:
    """Explicit sensitive disclosure; never expose as an unrestricted AI tool."""
    require_book(conn, profile_id)
    require_evidence(conn, profile_id, evidence_id)
    row = conn.execute("SELECT content,content_sha256 FROM gl_evidence WHERE profile_id=? AND id=?",
                       (profile_id, evidence_id)).fetchone()
    content = bytes(row["content"])
    if hashlib.sha256(content).hexdigest() != row["content_sha256"]:
        raise AppError("Evidence digest mismatch", code="accounting_evidence_corrupt")
    return content


def _sha256(value: Any) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise AppError("Expected canonical SHA-256 digest", code="accounting_invalid_input")
    return value


def _upload(conn: Any, profile_id: str, upload_id: str) -> dict:
    row = conn.execute("SELECT * FROM gl_evidence_uploads WHERE id=? AND profile_id=?", (upload_id, profile_id)).fetchone()
    if row is None:
        raise AppError("Upload not found in this book", code="accounting_upload_not_found")
    result = dict(row)
    result["received_bytes"] = result["total_bytes"] if result["evidence_id"] else conn.execute(
        "SELECT COALESCE(SUM(length(content)),0) FROM gl_evidence_upload_chunks WHERE upload_id=? AND profile_id=?",
        (upload_id, profile_id)).fetchone()[0]
    return result


def begin_upload(conn: Any, profile_id: str, *, name: str, media_type: str, total_bytes: int,
                 content_sha256: str, idempotency_key: str, source_document_id: str | None = None) -> dict:
    bounded_text(name, "name")
    bounded_text(media_type, "media_type", 128)
    bounded_text(idempotency_key, "idempotency_key", 128)
    _sha256(content_sha256)
    if type(total_bytes) is not int or not 0 < total_bytes <= MAX_EVIDENCE_BYTES:
        raise AppError("Evidence must contain at most 20 MiB", code="accounting_evidence_size")
    with atomic(conn):
        require_book(conn, profile_id)
        if source_document_id is not None and not conn.execute("SELECT 1 FROM external_documents WHERE id=? AND profile_id=?",
                                                               (source_document_id, profile_id)).fetchone():
            raise AppError("Document not found in this book", code="accounting_evidence_scope")
        prior = conn.execute("SELECT id FROM gl_evidence_uploads WHERE profile_id=? AND idempotency_key=?",
                             (profile_id, idempotency_key)).fetchone()
        if prior:
            record = _upload(conn, profile_id, prior["id"])
            if any(record[key] != value for key, value in {"name": name, "media_type": media_type,
                    "total_bytes": total_bytes, "content_sha256": content_sha256, "source_document_id": source_document_id}.items()):
                raise AppError("Upload identity reused with different content", code="accounting_idempotency_conflict")
        else:
            # Bound abandoned encrypted staging independently of individual file limits.
            count = conn.execute("SELECT COUNT(*) FROM gl_evidence_uploads WHERE profile_id=? AND evidence_id IS NULL", (profile_id,)).fetchone()[0]
            if count >= 10:
                raise AppError("Cancel unfinished evidence uploads before starting more", code="accounting_upload_limit")
            upload_id = str(uuid.uuid4())
            conn.execute("INSERT INTO gl_evidence_uploads VALUES (?,?,?,?,?,?,?,?,NULL)",
                         (upload_id, profile_id, name, media_type, total_bytes, content_sha256, idempotency_key, source_document_id))
            record = _upload(conn, profile_id, upload_id)
        return {"upload_id": record["id"], "received_bytes": record["received_bytes"],
                "total_bytes": record["total_bytes"], "evidence_id": record["evidence_id"]}


def append_upload(conn: Any, profile_id: str, *, upload_id: str, offset: int,
                  content: bytes, chunk_sha256: str) -> dict:
    _sha256(chunk_sha256)
    if type(offset) is not int or offset < 0 or not isinstance(content, bytes) or not 0 < len(content) <= MAX_UPLOAD_CHUNK_BYTES:
        raise AppError("Invalid evidence upload chunk", code="accounting_upload_chunk")
    if hashlib.sha256(content).hexdigest() != chunk_sha256:
        raise AppError("Chunk digest mismatch", code="accounting_upload_digest")
    with atomic(conn):
        require_book(conn, profile_id)
        record = _upload(conn, profile_id, upload_id)
        if record["evidence_id"]:
            raise AppError("Upload already finalized; retrieve its evidence receipt", code="accounting_upload_finished")
        prior = conn.execute("SELECT content,chunk_sha256 FROM gl_evidence_upload_chunks WHERE upload_id=? AND profile_id=? AND offset=?",
                             (upload_id, profile_id, offset)).fetchone()
        if prior:
            if prior["chunk_sha256"] != chunk_sha256 or bytes(prior["content"]) != content:
                raise AppError("Chunk identity reused with different content", code="accounting_idempotency_conflict")
        else:
            if offset != record["received_bytes"] or offset + len(content) > record["total_bytes"]:
                raise AppError("Chunk offset does not match contiguous upload position", code="accounting_upload_offset")
            conn.execute("INSERT INTO gl_evidence_upload_chunks VALUES (?,?,?,?,?)", (upload_id, profile_id, offset, content, chunk_sha256))
        return {"upload_id": upload_id, "received_bytes": _upload(conn, profile_id, upload_id)["received_bytes"]}


def finish_upload(conn: Any, profile_id: str, *, upload_id: str) -> dict:
    with atomic(conn):
        require_book(conn, profile_id)
        record = _upload(conn, profile_id, upload_id)
        if record["evidence_id"]:
            return require_evidence(conn, profile_id, record["evidence_id"])
        if record["received_bytes"] != record["total_bytes"]:
            raise AppError("Evidence upload is incomplete", code="accounting_upload_incomplete")
        content = b"".join(bytes(row[0]) for row in conn.execute(
            "SELECT content FROM gl_evidence_upload_chunks WHERE upload_id=? AND profile_id=? ORDER BY offset", (upload_id, profile_id)))
        if len(content) != record["total_bytes"] or hashlib.sha256(content).hexdigest() != record["content_sha256"]:
            raise AppError("Final evidence digest mismatch", code="accounting_upload_digest")
        result = retain_evidence(conn, profile_id, content=content, name=record["name"], media_type=record["media_type"],
                                 source_document_id=record["source_document_id"])
        conn.execute("UPDATE gl_evidence_uploads SET evidence_id=? WHERE id=? AND profile_id=?", (result["id"], upload_id, profile_id))
        conn.execute("DELETE FROM gl_evidence_upload_chunks WHERE upload_id=? AND profile_id=?", (upload_id, profile_id))
        return result


def cancel_upload(conn: Any, profile_id: str, *, upload_id: str) -> dict:
    with atomic(conn):
        require_book(conn, profile_id)
        record = _upload(conn, profile_id, upload_id)
        if record["evidence_id"]:
            raise AppError("Finalized evidence remains retained", code="accounting_evidence_retained")
        conn.execute("DELETE FROM gl_evidence_upload_chunks WHERE upload_id=? AND profile_id=?", (upload_id, profile_id))
        conn.execute("DELETE FROM gl_evidence_uploads WHERE id=? AND profile_id=?", (upload_id, profile_id))
        return {"upload_id": upload_id, "cancelled": True}


def list_uploads(conn: Any, profile_id: str) -> list[dict]:
    require_book(conn, profile_id)
    return [{"upload_id": row["id"], "name": row["name"], "total_bytes": row["total_bytes"],
             "received_bytes": _upload(conn, profile_id, row["id"])["received_bytes"]}
            for row in conn.execute("SELECT id,name,total_bytes FROM gl_evidence_uploads WHERE profile_id=? AND evidence_id IS NULL ORDER BY id LIMIT 10",
                                    (profile_id,)).fetchall()]
