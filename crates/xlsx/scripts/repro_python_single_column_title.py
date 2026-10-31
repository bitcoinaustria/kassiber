#!/usr/bin/env python3
"""Reproduce Python's omitted single-column report title on disposable data."""
from pathlib import Path
import tempfile
import warnings

import openpyxl
import xlsxwriter

from kassiber.core.reports import _generic_report_xlsx_formats, _generic_report_xlsx_write_sheet

ROOT = Path(__file__).resolve().parents[3]

with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="xlsx-title-repro-") as scratch:
    path = Path(scratch) / "one-column.xlsx"
    workbook = xlsxwriter.Workbook(path)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        _generic_report_xlsx_write_sheet(workbook, dict(sheet_name="Assumptions", title="Missing title",
            headers=["note"], rows=[dict(note="Review")]), _generic_report_xlsx_formats(workbook))
    workbook.close()
    book = openpyxl.load_workbook(path)
    assert book.active["A1"].value is None
    assert any("single cell" in str(w.message) for w in caught)
    book.close()
    print("Reproduced: Python warns about a one-cell merge and omits the sheet title.")
