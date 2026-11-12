"""A valid one-column workbook still displays its report title."""

import warnings

import openpyxl
import xlsxwriter

from kassiber.core.reports import _generic_report_xlsx_formats, _generic_report_xlsx_write_sheet


def test_single_column_report_keeps_its_title_without_a_merge_warning(tmp_path):
    path = tmp_path / "one-column.xlsx"
    with xlsxwriter.Workbook(path) as workbook, warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        _generic_report_xlsx_write_sheet(workbook, {"sheet_name": "Notes", "title": "Review notes", "headers": ["note"], "rows": [{"note": "Review"}]}, _generic_report_xlsx_formats(workbook))
    book = openpyxl.load_workbook(path)
    try:
        assert book.active["A1"].value == "Review notes"
    finally:
        book.close()
    assert not caught
