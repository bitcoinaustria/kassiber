"""Output and disclosure boundaries of an already reviewed case export."""

import hashlib
import json
import zipfile
from pathlib import Path
from unittest.mock import patch

import pytest

from kassiber.core import source_funds


def _saved_report(attachment: Path, *, reveal_mode: str = "full") -> dict:
    return {
        "explain_gates": {"exportable": True},
        "reveal_mode": reveal_mode,
        "target": {"transaction_id": "reviewed-target"},
        "disclosure_preview": {
            "attachments": [{
                "id": "reviewed-evidence",
                "label": "Reviewed proof",
                "sha256": hashlib.sha256(attachment.read_bytes()).hexdigest(),
                "media_type": "text/plain",
            }],
        },
    }


def _write_pdf(path, **kwargs):
    # PDF rendering is tested separately; these tests exercise archive writes.
    Path(path).write_bytes(b"%PDF-test")


def _export(root: Path, report: dict, resolved: dict):
    with (
        patch.object(source_funds, "load_case_snapshot", return_value=report),
        patch.object(source_funds, "write_source_funds_pdf", side_effect=_write_pdf),
        patch("kassiber.core.attachments.resolve_attachment_files", return_value=resolved),
    ):
        return source_funds.export_bundle(
            object(), None, None, str(root / "report.zip"), None,
            data_root=str(root), case_ref="reviewed-case",
        )


def test_archive_write_failure_preserves_existing_export_and_cleans_staging(tmp_path):
    attachment = tmp_path / "original.txt"
    attachment.write_bytes(b"Original reviewed bytes")
    report = _saved_report(attachment)
    output = tmp_path / "report.zip"
    previous = b"Previous completed archive"
    output.write_bytes(previous)

    with patch.object(zipfile.ZipFile, "write", side_effect=OSError("disk full")):
        with pytest.raises(OSError, match="disk full"):
            _export(tmp_path, report, {"reviewed-evidence": {"resolved_path": str(attachment)}})

    assert output.read_bytes() == previous
    assert not list(tmp_path.glob(".report.zip.*.tmp"))


@pytest.mark.parametrize("reveal_mode", ["minimal", "labels_only"])
def test_withheld_evidence_is_not_resolved_or_read(tmp_path, reveal_mode):
    attachment = tmp_path / "original.txt"
    attachment.write_bytes(b"Original reviewed bytes")
    report = _saved_report(attachment, reveal_mode=reveal_mode)
    attachment.unlink()
    with (
        patch.object(source_funds, "load_case_snapshot", return_value=report),
        patch.object(source_funds, "write_source_funds_pdf", side_effect=_write_pdf),
        patch(
            "kassiber.core.attachments.resolve_attachment_files",
            side_effect=AssertionError("Withheld evidence must not be resolved"),
        ),
    ):
        result = source_funds.export_bundle(
            object(), None, None, str(tmp_path / "report.zip"), None,
            data_root=str(tmp_path), case_ref="reviewed-case",
        )

    assert result["evidence_files"] == 0
    assert result["evidence_withheld"] == 1
    with zipfile.ZipFile(tmp_path / "report.zip") as archive:
        assert not any(name.startswith("evidence/") for name in archive.namelist())
        manifest = json.loads(archive.read("manifest.json"))
    assert manifest["evidence"][0]["source"] == "withheld_by_reveal_mode"


def test_full_disclosure_uses_saved_url_after_live_attachment_url_changes(tmp_path):
    saved_url = "https://original.example/reviewed-proof"
    replacement_url = "https://replacement.example/unreviewed-proof"
    report = {
        "explain_gates": {"exportable": True},
        "reveal_mode": "full",
        "target": {"transaction_id": "reviewed-target"},
        "disclosure_preview": {
            "attachments": [{"id": "url-proof", "label": "Proof", "source_url": saved_url}],
        },
    }
    _export(tmp_path, report, {"url-proof": {"url": replacement_url, "resolved_path": None}})

    with zipfile.ZipFile(tmp_path / "report.zip") as archive:
        manifest_bytes = archive.read("manifest.json")
        manifest = json.loads(manifest_bytes)
    assert manifest["evidence"][0]["source_url"] == saved_url
    assert replacement_url.encode() not in manifest_bytes
