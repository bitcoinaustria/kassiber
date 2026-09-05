import io
import json
import os
from pathlib import Path
import shutil
import signal
import struct
import subprocess
import sys
import threading
import time
from types import SimpleNamespace
import zlib

import pytest

from kassiber.core.accounting import document_text as text, evidence
from kassiber.core.accounting._document_ocr_worker import image_dimensions
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db  # noqa: F401


def png(width=120, height=60):
    def chunk(kind, content):
        return struct.pack(">I", len(content)) + kind + content + struct.pack(">I", zlib.crc32(kind + content))
    raw = (b"\0" + b"\xff" * width * 3) * height
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


@pytest.fixture
def fake_tesseract(tmp_path, monkeypatch):
    path = tmp_path / "tesseract"
    path.write_text(f"#!{sys.executable}\n" + '''
import os, sys
assert not any(key in os.environ for key in ('HOME','OPENAI_API_KEY','ANTHROPIC_API_KEY','TESSDATA_PREFIX','PYTHONPATH'))
if '--version' in sys.argv: print('tesseract 5.5 synthetic')
elif '--list-langs' in sys.argv: print('List of available languages (2):\\neng\\ndeu')
else:
 assert sys.argv[1:3] == ['stdin','stdout']
 assert 'stream_filelist=0' in sys.argv and 'tessedit_write_images=0' in sys.argv
 content=sys.stdin.buffer.read()
 assert content.startswith(b'\\x89PNG\\r\\n\\x1a\\n') or content.startswith(b'\\xff\\xd8')
 print('Synthetic invoice 42 EUR 120.00')
''', encoding="utf-8")
    path.chmod(0o755)
    actual = shutil.which
    monkeypatch.setattr(text.shutil, "which", lambda name: str(path) if name == "tesseract" else actual(name))
    return path


def test_explicit_image_ocr_is_retained_encrypted_and_has_no_plaintext_scratch(accounting_db, fake_tesseract, tmp_path, monkeypatch):
    text.ensure_schema(accounting_db)
    source = evidence.retain_evidence(accounting_db, "p", content=png(), name="Receipt", media_type="image/png")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("OPENAI_API_KEY", "test-token-placeholder")
    before = {p.name for p in tmp_path.iterdir()}
    result = text.extract(accounting_db, "p", evidence_id=source["id"], method="ocr", ocr_language="eng+deu")
    assert result["pages"] == ["Synthetic invoice 42 EUR 120.00"]
    assert result["method"] == "tesseract"
    assert "lang=eng+deu; pages=1" in result["tool_version"]
    assert before == {p.name for p in tmp_path.iterdir()}
    accounting_db.commit()
    assert b"Synthetic invoice 42" not in (tmp_path / "accounting.db").read_bytes()


def test_no_automatic_ocr_or_provider_fallback(fake_tesseract, monkeypatch):
    monkeypatch.setattr(text.subprocess, "Popen", lambda *a, **kw: pytest.fail("Default image extraction must not launch OCR"))
    with pytest.raises(AppError) as raised:
        text.extract_bytes(png(), "image/png")
    assert raised.value.code == "accounting_document_text_unsupported"
    assert text.extract_bytes(b"Plain receipt", "text/plain") == (["Plain receipt"], "utf8", "1")


@pytest.mark.parametrize("options", [
    {"method": []}, {"method": "shell"}, {"method": "text", "ocr_pages": [1]},
    {"method": "ocr", "ocr_pages": []}, {"method": "ocr", "ocr_pages": [True]},
    {"method": "ocr", "ocr_pages": [1, 1]}, {"method": "ocr", "ocr_pages": [2]},
    {"method": "ocr", "ocr_language": "../../file"}, {"method": "ocr", "ocr_language": "--help"},
])
def test_invalid_selections_fail_before_process(options, monkeypatch):
    monkeypatch.setattr(text.subprocess, "Popen", lambda *a, **kw: pytest.fail("Invalid OCR options launched a process"))
    with pytest.raises(AppError):
        text.extract_bytes(png(), "image/png", **options)


def test_missing_runtime_and_language_are_actionable_without_fallback(fake_tesseract, monkeypatch):
    with pytest.raises(AppError) as raised:
        text.extract_bytes(png(), "image/png", method="ocr", ocr_language="fra")
    assert raised.value.code == "accounting_ocr_language_unavailable"
    monkeypatch.setattr(text.shutil, "which", lambda _: None)
    with pytest.raises(AppError) as raised:
        text.extract_bytes(png(), "image/png", method="ocr")
    assert raised.value.code == "accounting_ocr_unavailable"
    assert text.capabilities()["ocr_images"] is False


@pytest.mark.parametrize("content", [b"https://example.invalid/image.png", b"/private/accounting.txt\n", b"<svg/>", png(1, 1)[:20], b"\xff\xd8\xff\xc0\x00\x01"])
def test_ocr_rejects_urls_file_lists_and_malformed_image_headers(content, fake_tesseract):
    with pytest.raises(AppError) as raised:
        text.extract_bytes(content, "image/png", method="ocr")
    assert raised.value.code == "accounting_document_parse_failed"


def test_image_pixel_budget_and_jpeg_header():
    assert image_dimensions(png(120, 60)) == (120, 60)
    assert image_dimensions(b"\xff\xd8\xff\xc0\x00\x0b\x08\x00\x3c\x00\x78\x01\x01\x11\x00") == (120, 60)
    excessive = bytearray(png(1, 1))
    excessive[16:24] = struct.pack(">II", 10000, 10000)
    with pytest.raises(ValueError, match="image_dimensions"):
        image_dimensions(bytes(excessive))


def test_unsupported_process_tree_isolation_fails_before_native_process(monkeypatch):
    monkeypatch.setattr(text, "os", SimpleNamespace(name="nt"))
    monkeypatch.setattr(text.subprocess, "Popen", lambda *a, **kw: pytest.fail("Unsupported isolation launched parser"))
    assert text.capabilities()["parser_isolation_supported"] is False
    assert text.capabilities()["pdf_text"] is False
    with pytest.raises(AppError) as raised:
        text._run_worker("ocr", [], png(), None, timeout=1)
    assert raised.value.code == "accounting_document_isolation_unavailable"


def test_ocr_output_budget_stops_flooding_native_process(fake_tesseract):
    source = fake_tesseract.read_text(encoding="utf-8")
    fake_tesseract.write_text(source.replace("print('Synthetic invoice 42 EUR 120.00')", "sys.stdout.write('x' * (2 * 1024**2 + 1))"), encoding="utf-8")
    with pytest.raises(AppError) as raised:
        text.extract_bytes(png(), "image/png", method="ocr")
    assert raised.value.code == "accounting_document_parse_failed"


def test_pdf_ocr_requires_explicit_pages_and_renderer(fake_tesseract, monkeypatch):
    with pytest.raises(AppError) as raised:
        text.extract_bytes(b"%PDF-1.4", "application/pdf", method="ocr")
    assert raised.value.code == "accounting_ocr_pages_invalid"
    monkeypatch.setattr(text.shutil, "which", lambda name: str(fake_tesseract) if name == "tesseract" else None)
    with pytest.raises(AppError) as raised:
        text.extract_bytes(b"%PDF-1.4", "application/pdf", method="ocr", ocr_pages=[1])
    assert raised.value.code == "accounting_ocr_pdf_renderer_unavailable"


def test_real_poppler_rasterizes_only_selected_pdf_page_through_pipes(fake_tesseract, tmp_path, monkeypatch):
    if not shutil.which("pdftoppm"):
        pytest.skip("Optional local Poppler renderer unavailable")
    from reportlab.pdfgen.canvas import Canvas
    out = io.BytesIO()
    canvas = Canvas(out)
    for page in (1, 2, 3):
        canvas.drawString(50, 700, f"Synthetic invoice page {page}")
        canvas.showPage()
    canvas.save()
    monkeypatch.chdir(tmp_path)
    before = {p.name for p in tmp_path.iterdir()}
    pages, method, version = text.extract_bytes(out.getvalue(), "application/pdf", method="ocr", ocr_pages=[2])
    assert pages == ["", "Synthetic invoice 42 EUR 120.00"]
    assert method == "tesseract" and "pdftoppm version" in version and "pages=2" in version
    assert before == {p.name for p in tmp_path.iterdir()}


@pytest.mark.skipif(os.name != "posix", reason="POSIX process-group cancellation")
def test_cancellation_kills_parser_process_group(fake_tesseract, monkeypatch):
    fake_tesseract.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(20)\n", encoding="utf-8")
    cancel = threading.Event()
    timer = threading.Timer(0.15, cancel.set)
    actual = os.killpg
    calls = []
    monkeypatch.setattr(text.os, "killpg", lambda pid, sig: (calls.append((pid, sig)), actual(pid, sig))[1])
    start = time.monotonic()
    timer.start()
    try:
        with pytest.raises(AppError) as raised:
            text.extract_bytes(png(), "image/png", method="ocr", cancel=cancel)
        assert raised.value.code == "accounting_document_cancelled"
        assert time.monotonic() - start < 3
        assert calls and calls[-1][1] == signal.SIGKILL
    finally:
        timer.cancel()


def test_frozen_worker_command_uses_fixed_sidecar_entrypoint(monkeypatch):
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    assert text._worker_command("ocr", ["tesseract", "", "image/png", "eng", "[1]"]) == [sys.executable, "--accounting-document-worker", "ocr", "tesseract", "", "image/png", "eng", "[1]"]
    with pytest.raises(AppError):
        text._worker_command("arbitrary_module", [])


def test_frozen_bootstrap_dispatches_before_daemon_import(fake_tesseract):
    script = """
import importlib.abc, sys
class NoDaemon(importlib.abc.MetaPathFinder):
 def find_spec(self, fullname, path=None, target=None):
  if fullname == 'kassiber.daemon': raise AssertionError('must not import daemon for parser job')
sys.meta_path.insert(0, NoDaemon())
sys.frozen = True
sys.argv = ['kassiber', '--accounting-document-worker', 'ocr', *sys.argv[1:]]
import kassiber.cli.main
"""
    result = subprocess.run([sys.executable, "-c", script, str(fake_tesseract), "", "image/png", "eng", "[1]"],
        input=png(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10,
        env={"PATH": os.defpath, "LANG": "C", "LC_ALL": "C"})
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["pages"] == ["Synthetic invoice 42 EUR 120.00"]


def test_real_tesseract_ocr_offline_when_installed():
    if not shutil.which("tesseract"):
        pytest.skip("Optional local Tesseract is not installed; no automatic installation")
    from PIL import Image, ImageDraw, ImageFont
    image = Image.new("RGB", (1200, 240), "white")
    draw = ImageDraw.Draw(image)
    draw.text((30, 60), "INVOICE 42 EUR 120.00", fill="black", font=ImageFont.load_default(size=60))
    output = io.BytesIO()
    image.save(output, format="PNG")
    pages, method, _ = text.extract_bytes(output.getvalue(), "image/png", method="ocr")
    assert method == "tesseract" and "120.00" in pages[0] and "42" in pages[0]
