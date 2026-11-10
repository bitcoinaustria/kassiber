"""XLSX renderer content parity; Python remains the independent report oracle."""
from dataclasses import replace
from datetime import date, datetime
import json
import os
from pathlib import Path
import subprocess
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, canonical, first_difference, STAMP

ROOT = Path(__file__).resolve().parents[1]
BIN = os.environ.get("KASSIBER_XLSX_RENDER_BIN")
pytestmark = pytest.mark.skipif(not BIN, reason="requires native XLSX example binary")


def compare(tmp_path, write):
    python = tmp_path / "python.xlsx"
    rust = tmp_path / "rust.xlsx"
    workbooks = []
    def factory(*args, **kwargs):
        book = RecordingWorkbook(*args, **kwargs)
        workbooks.append(book)
        return book
    with patch("xlsxwriter.Workbook", factory):
        write(python)
    assert len(workbooks) == 1
    document = workbooks[0].document
    request = json.dumps(dict(document=document, path=str(rust))) + "\n"
    result = subprocess.run([BIN], input=request, text=True, capture_output=True, check=True)
    assert json.loads(result.stdout) == {"ok": True}, result.stdout
    assert not result.stderr
    expected = canonical(python)
    actual = canonical(rust)
    assert not (difference := first_difference(expected, actual)), difference
    before = rust.read_bytes()
    subprocess.run([BIN], input=request, text=True, capture_output=True, check=True)
    assert before == rust.read_bytes(), "nondeterministic workbook bytes"
    return document


@pytest.mark.parametrize("kind", ["full", "transactions", "austrian", "handoff", "exit"])
def test_renderer_contract_workbooks(tmp_path, kind):
    from kassiber.core import year_handoff
    conn = open_db(tmp_path / "data")
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text())
    hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
    if kind in {"austrian", "exit"}:
        conn.execute("UPDATE profiles SET tax_country='at' WHERE id='p'")
        conn.commit()
    if kind == "handoff":
        conn.execute("DELETE FROM journal_quarantines")
        conn.commit()
    calls = {
        "full": lambda p: reports.export_xlsx_report(conn, None, None, p, hooks),
        "transactions": lambda p: reports.export_transactions_xlsx_report(conn, None, None, p, hooks),
        "austrian": lambda p: reports.export_austrian_e1kv_xlsx_report(conn, None, None, p, hooks, tax_year=2025),
        "handoff": lambda p: year_handoff.export_year_handoff_xlsx(conn, None, None, p, hooks, year=2025),
        "exit": lambda p: reports.export_exit_tax_xlsx_report(conn, None, None, p, hooks,
                                                              departure_date="2026-01-02", destination="third_country"),
    }
    try:
        compare(tmp_path, calls[kind])
    finally:
        conn.close()


@pytest.mark.parametrize("value", [None, "", True, False, 0, -12345.678901234567, 1e-100, 1e100,
    "日本語 € 😀", "line\n" + "long " * 8000, "=SUM(A1:A3)", "https://example.invalid/inert"])
def test_renderer_edge_cells(tmp_path, value):
    def write(path):
        import xlsxwriter
        book = xlsxwriter.Workbook(path)
        formats = reports._generic_report_xlsx_formats(book)
        reports._generic_report_xlsx_write_sheet(book, dict(sheet_name="Unicode €", title="Edges",
            headers=["value", "quantity", "cost_basis"], rows=[dict(value=value, quantity=value, cost_basis=value)]), formats)
        book.define_name("Tolerance", "='Unicode €'!$A$3")
        book.close()
    compare(tmp_path, write)


def test_renderer_many_rows(tmp_path):
    def write(path):
        import xlsxwriter
        book = xlsxwriter.Workbook(path)
        reports._generic_report_xlsx_write_sheet(book, dict(sheet_name="Data", title="Rows",
            headers=["count", "cost_basis"], rows=[dict(count=i, cost_basis=i/-100) for i in range(2000)]),
            reports._generic_report_xlsx_formats(book))
        book.close()
    compare(tmp_path, write)


CASES = json.loads((ROOT / "crates/reports/tests/data/artifact_csv.json").read_text())


@pytest.mark.parametrize("case", CASES, ids=lambda c: c["name"])
def test_renderer_export_fixture_corpus(tmp_path, case):
    """Every inherited export fixture, including admission refusals.

    This exercises the independent Python report projection and Rust rendering;
    it does not claim native report-admission parity for unregistered commands.
    """
    from kassiber.core.export_range import ExportRange
    conn = open_db(tmp_path / "data")
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + case["sql"])
    hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
    args = json.loads(case["args_json"])
    command = case["command"]
    if command == "reports.export-csv":
        write = lambda p: reports.export_xlsx_report(conn, None, None, p, hooks,
            wallet_ref=args.get("wallet"), history_limit=args.get("history_limit"))
    elif command == "reports.export-transactions-csv":
        period = args.get("export_range")
        if period:
            period = ExportRange(preset=period["preset"], timezone=period["timezone"],
                start=datetime.fromisoformat(period["start"].replace("Z", "+00:00")),
                end=datetime.fromisoformat(period["end"].replace("Z", "+00:00")),
                first_day=date.fromisoformat(period["first_day"]), last_day=date.fromisoformat(period["last_day"]))
        write = lambda p: reports.export_transactions_xlsx_report(conn, None, None, p, hooks,
            wallet_ref=args.get("wallet"), wallet_refs=args.get("wallets"), export_range=period)
    else:
        write = lambda p: reports.export_austrian_e1kv_xlsx_report(conn, None, None, p, hooks, tax_year=args.get("year"))
    try:
        if "error" in case["expected"]:
            with pytest.raises((AppError, ValueError, TypeError, OverflowError, AttributeError)) as caught:
                write(tmp_path / "refused.xlsx")
            expected = case["expected"]["error"]
            assert str(caught.value) == expected["message"]
            assert not (tmp_path / "refused.xlsx").exists()
        else:
            compare(tmp_path, write)
    finally:
        conn.close()


@pytest.mark.parametrize("tool", ["cointracking", "blockpit"])
def test_renderer_cross_check(tmp_path, tool):
    from kassiber.core import cross_check
    from kassiber.importers import _COINTRACKING_REQUIRED_COLUMNS, _BLOCKPIT_REQUIRED_COLUMNS
    import csv
    conn = open_db(tmp_path / "data")
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text())
    source = tmp_path / "source.csv"
    with source.open("w", newline="") as handle:
        csv.writer(handle).writerow(_COINTRACKING_REQUIRED_COLUMNS if tool == "cointracking" else _BLOCKPIT_REQUIRED_COLUMNS)
    try:
        compare(tmp_path, lambda p: cross_check.export_cross_check_xlsx(conn, None, None, p, _report_hooks(),
            tool=tool, file_path=source))
    finally:
        conn.close()
