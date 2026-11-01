#!/usr/bin/env python3
"""Verify Python's corrected single-column report title on disposable data."""
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
        _generic_report_xlsx_write_sheet(workbook, dict(sheet_name="Assumptions", title="Assumptions",
            headers=["note"], rows=[dict(note="Review")]), _generic_report_xlsx_formats(workbook))
    workbook.close()
    book = openpyxl.load_workbook(path)
    assert book.active["A1"].value == "Assumptions"
    assert not caught
    book.close()
    print("Verified: the single-column sheet has a title without merge warnings.")
