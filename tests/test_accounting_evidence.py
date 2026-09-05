import sqlite3
import hashlib

import pytest

from kassiber.core.accounting import bank, evidence, ledger, schedules, schema
from kassiber.errors import AppError


@pytest.fixture
def accounting_db(tmp_path):
    sqlcipher = pytest.importorskip("sqlcipher3")
    conn = sqlcipher.connect(str(tmp_path / "accounting.db"))
    conn.row_factory = sqlcipher.Row
    conn.execute("PRAGMA key='synthetic-test-only'")
    conn.execute("PRAGMA kdf_iter=4000")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("CREATE TABLE profiles(id TEXT PRIMARY KEY)")
    conn.execute("CREATE TABLE external_documents(id TEXT PRIMARY KEY,profile_id TEXT NOT NULL)")
    conn.executemany("INSERT INTO profiles VALUES (?)", [("p",), ("other",)])
    schema.ensure_schema(conn)
    evidence.ensure_schema(conn)
    bank.ensure_schema(conn)
    schedules.ensure_schema(conn)
    for profile in ("p", "other"):
        ledger.configure_book(conn, profile, currency="EUR", timezone="Europe/Vienna")
        for code, kind in (("bank", "asset"), ("ar", "asset"), ("ap", "liability"),
                           ("sales", "income"), ("expense", "expense"), ("equity", "equity")):
            ledger.create_account(conn, profile, code=code, name=code, kind=kind)
        ledger.create_period(conn, profile, period_id="2026", start_date="2026-01-01", end_date="2026-12-31")
    conn.commit()
    yield conn
    conn.close()


def post(conn, debit, credit, amount=100, *, key="posting", day="2026-03-01", profile="p"):
    draft = ledger.create_draft(conn, profile, {"idempotency_key": key, "period_id": "2026",
        "entry_date": day, "description": "Synthetic entry", "lines": [
            {"account_code": debit, "debit_minor": amount},
            {"account_code": credit, "credit_minor": amount}]})
    return ledger.post_draft(conn, profile, draft_id=draft["id"], expected_digest=draft["payload_digest"])


def retained(conn, profile="p"):
    return evidence.retain_evidence(conn, profile, content=b"Synthetic invoice", media_type="text/plain", name="invoice")["id"]


def test_evidence_encrypted_retained_scoped(accounting_db, tmp_path):
    conn = accounting_db
    identifier = retained(conn)
    assert evidence.read_evidence_bytes(conn, "p", identifier) == b"Synthetic invoice"
    assert "content" not in evidence.list_evidence(conn, "p")[0]
    assert evidence.list_evidence(conn, "other") == []
    with pytest.raises(AppError):
        evidence.read_evidence_bytes(conn, "other", identifier)
    with pytest.raises(Exception, match="accounting_evidence_retained"):
        conn.execute("DELETE FROM gl_evidence WHERE id=?", (identifier,))
    with pytest.raises(Exception, match="accounting_evidence_retained"):
        conn.execute("UPDATE gl_evidence SET content=x'00' WHERE id=?", (identifier,))
    with pytest.raises(Exception, match="accounting_evidence_retained"):
        conn.execute("INSERT OR REPLACE INTO gl_evidence SELECT * FROM gl_evidence WHERE id=?", (identifier,))
    conn.commit()
    assert b"Synthetic invoice" not in (tmp_path / "accounting.db").read_bytes()


def test_evidence_rejects_plaintext_and_cross_document(accounting_db):
    plain = sqlite3.connect(":memory:")
    with pytest.raises(AppError, match="encrypted"):
        evidence.retain_evidence(plain, "p", content=b"secret", media_type="text/plain", name="secret")
    plain.close()
    conn = accounting_db
    conn.execute("INSERT INTO external_documents VALUES ('otherdoc','other')")
    with pytest.raises(AppError):
        evidence.retain_evidence(conn, "p", content=b"x", media_type="text/plain", name="x", source_document_id="otherdoc")
    conn.execute("INSERT INTO external_documents VALUES ('doc','p')")
    evidence.retain_evidence(conn, "p", content=b"x", media_type="text/plain", name="x", source_document_id="doc")
    with pytest.raises(Exception, match="accounting_evidence_retained"):
        conn.execute("DELETE FROM external_documents WHERE id='doc'")


@pytest.mark.parametrize("content", [b"", "not bytes", b"x" * (evidence.MAX_EVIDENCE_BYTES + 1)], ids=["empty", "text", "oversized"])
def test_evidence_size(accounting_db, content):
    with pytest.raises(AppError):
        evidence.retain_evidence(accounting_db, "p", content=content, media_type="text/plain", name="x")


def test_schema_does_not_commit(accounting_db):
    conn = accounting_db
    conn.execute("INSERT INTO external_documents VALUES ('rollback','p')")
    evidence.ensure_schema(conn)
    bank.ensure_schema(conn)
    schedules.ensure_schema(conn)
    conn.rollback()
    assert not conn.execute("SELECT 1 FROM external_documents WHERE id='rollback'").fetchone()


def test_chunked_upload_exact_scoped_retry_and_finalize(accounting_db, tmp_path):
    conn = accounting_db
    content = b"private-chunk-evidence-marker" * 20000
    args = dict(name="File", media_type="application/octet-stream", total_bytes=len(content),
                content_sha256=hashlib.sha256(content).hexdigest(), idempotency_key="upload-1")
    begun = evidence.begin_upload(conn, "p", **args)
    assert evidence.begin_upload(conn, "p", **args) == begun
    with pytest.raises(AppError):
        evidence.begin_upload(conn, "p", **{**args, "name": "Changed"})
    with pytest.raises(AppError):
        evidence.finish_upload(conn, "p", upload_id=begun["upload_id"])
    for offset in range(0, len(content), evidence.MAX_UPLOAD_CHUNK_BYTES):
        chunk = content[offset:offset + evidence.MAX_UPLOAD_CHUNK_BYTES]
        args_chunk = dict(upload_id=begun["upload_id"], offset=offset, content=chunk, chunk_sha256=hashlib.sha256(chunk).hexdigest())
        result = evidence.append_upload(conn, "p", **args_chunk)
        assert evidence.append_upload(conn, "p", **args_chunk) == result
        with pytest.raises(AppError):
            evidence.append_upload(conn, "other", **args_chunk)
    conn.commit()
    assert b"private-chunk-evidence-marker" not in (tmp_path / "accounting.db").read_bytes()
    result = evidence.finish_upload(conn, "p", upload_id=begun["upload_id"])
    assert evidence.finish_upload(conn, "p", upload_id=begun["upload_id"])["id"] == result["id"]
    assert evidence.read_evidence_bytes(conn, "p", result["id"]) == content
    assert conn.execute("SELECT COUNT(*) FROM gl_evidence_upload_chunks").fetchone()[0] == 0
    with pytest.raises(AppError):
        evidence.cancel_upload(conn, "p", upload_id=begun["upload_id"])
    with pytest.raises(Exception, match="immutable"):
        conn.execute("UPDATE gl_evidence_uploads SET name='changed' WHERE id=?", (begun["upload_id"],))


def test_chunked_upload_cancellation_and_bad_chunks(accounting_db):
    conn = accounting_db
    content = b"test"
    upload = evidence.begin_upload(conn, "p", name="File", media_type="text/plain", total_bytes=4,
        content_sha256=hashlib.sha256(content).hexdigest(), idempotency_key="upload")
    args = dict(upload_id=upload["upload_id"], offset=0, content=content, chunk_sha256=hashlib.sha256(content).hexdigest())
    for changed in ({"offset": 1}, {"chunk_sha256": "0" * 64}, {"content": b"x" * (evidence.MAX_UPLOAD_CHUNK_BYTES + 1)}):
        with pytest.raises(AppError):
            evidence.append_upload(conn, "p", **{**args, **changed})
    evidence.append_upload(conn, "p", **args)
    assert evidence.list_uploads(conn, "p")[0]["received_bytes"] == 4
    assert evidence.list_uploads(conn, "other") == []
    with pytest.raises(AppError):
        evidence.cancel_upload(conn, "other", upload_id=upload["upload_id"])
    evidence.cancel_upload(conn, "p", upload_id=upload["upload_id"])
    assert evidence.list_uploads(conn, "p") == []
    assert conn.execute("SELECT COUNT(*) FROM gl_evidence_upload_chunks").fetchone()[0] == 0
    assert conn.execute("SELECT COUNT(*) FROM gl_evidence").fetchone()[0] == 0
