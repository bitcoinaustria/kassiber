"""Reproduce retained Python cross-check quirks on disposable synthetic books."""
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.db import open_db
from crates.reports.scripts.gen_cross_check_vectors import cases, outcome


def main():
    scratch = ROOT / 'target/r5-reports16/reference-repro'
    scratch.mkdir(parents=True, exist_ok=True)
    wanted = {'invalid-interior-date', 'excluded-id-asset-blind', 'folded-fee',
              'nonfinite-Buy Amount-NaN', 'nonfinite-Buy Value-NaN', 'baseline',
              'journal-proceeds-NaN12',
              'nan-valued-negative-payload',
              'bare-cr-lines',
              'probable-date-bound-0001-01-01T00:00:00Z'}
    results = {}
    for case in cases():
        if case['name'] not in wanted:
            continue
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            directory = Path(directory)
            conn = open_db(directory / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + case['sql'])
            path = directory / 'export.csv'
            path.write_text(case['csv'], newline='')
            args = json.loads(case['args_json'])
            report = outcome(conn, args, path)
            name = case['name']
            if name == 'bare-cr-lines':
                results['bare_cr_error'] = report['error']['message']
                assert results['bare_cr_error'] == "new-line character seen in unquoted field - do you need to open the file with newline=''?"
            elif name == 'invalid-interior-date':
                results['malformed_stored_date'] = next(row['kassiber']['occurred_at'] for row in report['ok']['rows'] if row['kassiber'] and row['kassiber']['ref'] == 'out')
                assert results['malformed_stored_date'] == '1-01-01T00:00:00Z'
            elif name == 'excluded-id-asset-blind':
                results['excluded_duplicate_id_status'] = next(row['status'] for row in report['ok']['rows'] if row['tool'])
                assert results['excluded_duplicate_id_status'] == 'only_tool'
            elif name == 'folded-fee':
                results['folded_fee_message'] = next(d['message'] for row in report['ok']['rows'] for d in row['differences'] if d['field'] == 'fee')
                assert results['folded_fee_message'] == 'Same total, but the fee is split differently'
            elif name == 'nonfinite-Buy Amount-NaN':
                results['nan_amount_error'] = report['error']['message']
                assert results['nan_amount_error'] == 'cannot convert NaN to integer'
            elif name.startswith('probable-date-bound-'):
                results['probable_window_error'] = report['error']['message']
                assert results['probable_window_error'] == 'date value out of range'
            elif name == 'nan-valued-negative-payload':
                results['negative_nan_cash_read'] = next(row['tool']['value'] for row in report['ok']['rows'] if row['tool'])
                assert results['negative_nan_cash_read'] == '-NaN12'
            elif name == 'journal-proceeds-NaN12':
                results['journal_nan_cash_read'] = next(row['kassiber']['value'] for row in report['ok']['rows'] if row['kassiber'] and row['kassiber']['ref'] == 'no-tax')
                exported = outcome(conn, args, path, directory / 'report.xlsx')
                results['journal_nan_cash_xlsx_error'] = exported['error']['message']
                assert results['journal_nan_cash_read'] == 'NaN12'
                assert 'NAN/INF not supported' in results['journal_nan_cash_xlsx_error']
            else:
                target = directory / 'report.xlsx'
                exported = outcome(conn, args, path, target)
                if name == 'baseline':
                    results['registered_snapshots'] = conn.execute('SELECT count(*) FROM filed_report_snapshots').fetchone()[0]
                    assert 'ok' in exported and results['registered_snapshots'] == 0
                else:
                    results['nan_cash_read'] = next(row['tool']['value'] for row in report['ok']['rows'] if row['tool'])
                    results['nan_cash_xlsx_error'] = exported['error']['message']
                    assert results['nan_cash_read'] == 'NaN' and 'NAN/INF not supported' in results['nan_cash_xlsx_error']
            conn.close()
    assert len(results) == 12
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
