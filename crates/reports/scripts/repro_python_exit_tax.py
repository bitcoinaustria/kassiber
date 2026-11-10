import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch
import warnings
import openpyxl

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import exit_tax, reports
from kassiber.db import open_db
from crates.reports.scripts.gen_exit_tax_vectors import BASE, FixedDateTime, cases

scratch = ROOT / 'target/r5-reports14/exit-reference-repro'
scratch.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(dir=scratch) as path:
    path = Path(path)
    conn = open_db(path / 'data')
    history = next(sql for name, _, sql in cases() if name == 'amendment-history')
    conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + BASE + history +
        "UPDATE profiles SET gains_algorithm='HIFO'; UPDATE journal_wallet_holdings SET quantity=500000000000 WHERE id='wh';")
    with patch.object(exit_tax, 'datetime', FixedDateTime), warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter('always')
        report = reports.report_exit_tax(conn, None, None, _report_hooks(), departure_date='2024-01-01')
        output = reports.export_exit_tax_xlsx_report(conn, None, None, path / 'exit.xlsx', _report_hooks(), departure_date='2024-01-01')
    conn.execute("UPDATE journal_entries SET occurred_at='2024-01-01T00:30:00+02:00' WHERE id='new'")
    offset = reports.report_exit_tax(conn, None, None, _report_hooks(), departure_date='2023-12-31')
    workbook = openpyxl.load_workbook(path / 'exit.xlsx')
    print(json.dumps(dict(single_cell_merge_warning=[str(w.message) for w in caught],
        assumptions_title=workbook['Assumptions']['A1'].value,
        quarantine_disclosed_in_workbook=any('quarantin' in str(c.value).lower() for sheet in workbook for row in sheet for c in row),
        offset_held_neu_sats=offset['totals']['neuQuantitySats'], configured_method='HIFO', reported_method=report['method'],
        departure=report['departureDate'], transaction_fallback=report['fmvSource'],
        context_holdings=report['walletHoldings'], snapshot_registered='report_snapshot' in output,
        pending_impacts=conn.execute('SELECT COUNT(*) FROM custody_filed_report_impacts').fetchone()[0],
        amendment_disclosed='amendment' in json.dumps(report).lower()), sort_keys=True))
    workbook.close()
    conn.close()
