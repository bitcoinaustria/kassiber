"""Optional real-artifact smoke; build the normal sidecar and select it explicitly."""
import io
import json
import os
from pathlib import Path
import shutil
import subprocess

import pytest

from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def sidecar():
    configured = os.environ.get('KASSIBER_FROZEN_SMOKE_BIN')
    if not configured:
        pytest.skip('Set KASSIBER_FROZEN_SMOKE_BIN to an explicitly built sidecar')
    binary = Path(configured)
    assert binary.is_absolute() and binary.is_file()
    return str(binary)


def test_real_frozen_pdf_worker_uses_pipes_and_bundled_bootstrap(sidecar, tmp_path):
    from reportlab.pdfgen import canvas
    extractor = shutil.which('pdftotext')
    if not extractor:
        pytest.skip('Optional Poppler is not installed')
    source = io.BytesIO()
    pdf = canvas.Canvas(source)
    pdf.drawString(50, 700, 'Synthetic packaged invoice 42 EUR 120.00')
    pdf.save()
    result = subprocess.run([sidecar, '--accounting-document-worker', 'text', extractor],
        input=source.getvalue(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=tmp_path,
        env={'PATH': os.defpath, 'LANG': 'C', 'LC_ALL': 'C'}, timeout=30)
    assert result.returncode == 0, result.stderr
    pages = json.loads(result.stdout)['pages']
    assert any('Synthetic packaged invoice 42 EUR 120.00' in page for page in pages)
    assert list(tmp_path.iterdir()) == []


def test_real_frozen_tax_pack_data_and_encrypted_book_command(sidecar, book):
    conn, _, root = book
    conn.commit()
    read_fd, write_fd = os.pipe()
    try:
        os.write(write_fd, b'test-token-placeholder\n')
        os.close(write_fd)
        result = subprocess.run([sidecar, '--data-root', str(root), '--db-passphrase-fd', str(read_fd),
            '--machine', 'accounting', 'tax-packs', '--workspace', 'Test', '--profile', 'Test book', '--payload-stdin'],
            input=b'{}', stdout=subprocess.PIPE, stderr=subprocess.PIPE, pass_fds=(read_fd,), timeout=30)
    finally:
        os.close(read_fd)
    assert result.returncode == 0, result.stderr
    response = json.loads(result.stdout)
    assert response['kind'] == 'accounting.tax-packs'
    assert '2025' in result.stdout.decode()
    assert 'K2' in result.stdout.decode()
