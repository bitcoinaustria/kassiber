#!/usr/bin/env python3
"""Freeze independent Python Austrian workbook projections on disposable books."""
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook

OUT = ROOT / "crates/reports/tests/data/austrian_xlsx.json"


def build():
    cases = [c for c in json.loads((ROOT / "crates/reports/tests/data/artifact_csv.json").read_text())
             if c["command"] == "reports.export-austrian-e1kv-csv"]
    cases += [dict(name="xlsx-" + name, args_json='{"year":2025}', sql=sql) for name, sql in [
        ("mismatch", "UPDATE profiles SET tax_country='at'; UPDATE journal_entries SET at_kennzahl=175 WHERE id='income';"),
        ("unicode-long", "UPDATE profiles SET tax_country='at'; UPDATE wallets SET label='日本語 € 😀 ' || label; UPDATE journal_entries SET description='" + "long " * 100 + "';"),
    ]]
    result = []
    for case in cases:
        with tempfile.TemporaryDirectory(prefix="austrian-xlsx-") as root:
            conn = open_db(Path(root) / "data")
            conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + case["sql"])
            args = json.loads(case["args_json"])
            try:
                context = reports._require_report_context(conn, args.get("workspace"), args.get("profile"), _report_hooks())
                report = reports.report_austrian_e1kv(conn, args.get("workspace"), args.get("profile"),
                    _report_hooks(), tax_year=args.get("year"), report_context=context)
                book = RecordingWorkbook(Path(root) / "oracle.xlsx")
                formats = reports._austrian_e1kv_xlsx_formats(book)
                reports._austrian_e1kv_xlsx_write_overview(report, book, formats)
                reports._austrian_e1kv_xlsx_write_section_sheets(report, book, formats)
                reports._austrian_e1kv_xlsx_write_explanations(report, book, formats)
                book.close()
                expected = dict(document=book.document)
            except AppError as e:
                expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as e:
                expected = dict(error=dict(code="internal_error", message=str(e), hint=None, details=None, retryable=False))
            result.append(dict(name=case["name"], sql=case["sql"], args_json=case["args_json"], expected=expected))
            conn.close()
    return json.dumps(result, ensure_ascii=True, indent=2) + "\n"


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "Austrian XLSX vectors drifted"
    else:
        OUT.write_text(content)
