import pytest

from kassiber.core.accounting import document_text as text, evidence
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db  # noqa: F401


@pytest.fixture
def extracted(accounting_db):
    text.ensure_schema(accounting_db)
    source = evidence.retain_evidence(accounting_db, "p", content=b"Invoice 42\nTotal EUR 120.00",
        name="Invoice", media_type="text/plain")
    return text.extract(accounting_db, "p", evidence_id=source["id"])


def test_extraction_is_local_scoped_and_retained(accounting_db, extracted, tmp_path, monkeypatch):
    monkeypatch.setattr(text.subprocess, "Popen", lambda *a, **kw: pytest.fail("text extraction must not launch anything"))
    record = text.extract(accounting_db, "p", evidence_id=extracted["evidence_id"])
    assert record["pages"] == ["Invoice 42\nTotal EUR 120.00"]
    assert record["method"] == "utf8"
    with pytest.raises(AppError):
        text.get(accounting_db, "other", extraction_id=record["id"])
    for sql in ("DELETE FROM gl_evidence_extractions", "UPDATE gl_evidence_extractions SET pages_json='[]'",
                "INSERT OR REPLACE INTO gl_evidence_extractions SELECT * FROM gl_evidence_extractions"):
        with pytest.raises(Exception, match="accounting_evidence_retained"):
            accounting_db.execute(sql)
    accounting_db.commit()
    assert b"Invoice 42" not in (tmp_path / "accounting.db").read_bytes()


def test_reviews_preserve_source_and_require_current_revision(accounting_db, extracted):
    args = {"extraction_id": extracted["id"], "expected_digest": extracted["content_digest"],
        "previous_id": None, "fields": {"document_number": "42", "net_minor": 10000,
        "vat_minor": 2000, "total_minor": 12000, "currency": "EUR", "minor_unit_exponent": 2},
        "spans": {"document_number": {"page": 1, "start": 8, "end": 10}}, "reason": "Checked against source"}
    reviewed = text.review_fields(accounting_db, "p", **args)
    assert reviewed["review"]["fields"]["total_minor"] == 12000
    with pytest.raises(AppError) as exc:
        text.review_fields(accounting_db, "p", **args)
    assert exc.value.code == "accounting_stale_approval"
    corrected = text.review_fields(accounting_db, "p", **dict(args, previous_id=reviewed["review"]["id"],
        fields={"document_number": "42a"}, reason="Human correction"))
    assert corrected["review"]["previous_id"] == reviewed["review"]["id"]
    assert accounting_db.execute("SELECT COUNT(*) FROM gl_evidence_field_reviews").fetchone()[0] == 2


@pytest.mark.parametrize("fields,spans", [
    ({"total_minor": True}, {}), ({"total_minor": 1.5}, {}),
    ({"net_minor": 100, "vat_minor": 20, "total_minor": 119}, {}),
    ({"issued_date": "2025-02-30"}, {}), ({"currency": "eur"}, {}),
    ({"shell": "do something"}, {}),
    ({"document_number": "42"}, {"document_number": {"page": 0, "start": 0, "end": 2}}),
    ({"document_number": "42"}, {"document_number": {"page": 1, "start": 0, "end": 999}}),
])
def test_invalid_review_never_mutates(accounting_db, extracted, fields, spans):
    with pytest.raises(AppError):
        text.review_fields(accounting_db, "p", extraction_id=extracted["id"],
            expected_digest=extracted["content_digest"], previous_id=None, fields=fields,
            spans=spans, reason="test")
    assert accounting_db.execute("SELECT COUNT(*) FROM gl_evidence_field_reviews").fetchone()[0] == 0


def test_search_is_literal_scoped_and_metadata_only(accounting_db, extracted):
    found = text.search(accounting_db, "p", query="Invoice")
    assert found["extractions"][0]["id"] == extracted["id"]
    assert "pages" not in found["extractions"][0]
    assert text.search(accounting_db, "other", query="Invoice")["extractions"] == []
    assert text.search(accounting_db, "p", query="% OR 1=1")["extractions"] == []


def test_manual_transcription_is_honest_and_bounded(accounting_db, extracted):
    result = text.transcribe(accounting_db, "p", evidence_id=extracted["evidence_id"],
        pages=["Checked transcription"], reason="Human entered from image")
    assert result["method"] == "manual"
    with pytest.raises(AppError):
        text.transcribe(accounting_db, "p", evidence_id=extracted["evidence_id"], pages=["\x00"], reason="test")


def test_pdf_missing_never_falls_back_to_hosted(accounting_db, extracted, monkeypatch):
    monkeypatch.setattr(text.shutil, "which", lambda _: None)
    source = evidence.retain_evidence(accounting_db, "p", content=b"%PDF-1.4 synthetic invalid",
        name="PDF", media_type="application/pdf")
    with pytest.raises(AppError) as exc:
        text.extract(accounting_db, "p", evidence_id=source["id"])
    assert exc.value.code == "accounting_pdf_text_unavailable"


def test_real_pdf_uses_pipes_no_plaintext_files(accounting_db, monkeypatch):
    if not text.shutil.which("pdftotext"):
        pytest.skip("Optional local PDF text tool unavailable")
    import io
    from reportlab.pdfgen.canvas import Canvas
    out = io.BytesIO()
    canvas = Canvas(out)
    canvas.drawString(50, 700, "Synthetic receipt EUR 123.45")
    canvas.save()
    text.ensure_schema(accounting_db)
    source = evidence.retain_evidence(accounting_db, "p", content=out.getvalue(),
        name="PDF", media_type="application/pdf")
    record = text.extract(accounting_db, "p", evidence_id=source["id"])
    assert "Synthetic receipt EUR 123.45" in record["pages"][0]
    assert record["method"] == "pdftotext"
    assert record["tool_version"].startswith("pdftotext version")
