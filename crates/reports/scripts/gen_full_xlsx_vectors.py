import json
from dataclasses import replace
from datetime import datetime, timezone
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports, report_verify
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, STAMP

OUT = ROOT / 'crates/reports/tests/data/full_xlsx.json'
META = ROOT / 'crates/reports/src/verification_metadata.json'


class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc).astimezone(tz)


def metadata(root):
    book = RecordingWorkbook(root / 'styles.xlsx')
    styles = report_verify._build_formats(book)
    ref = dict(sheet_name='Acquisitions', col_index={c['key']: i for i, c in enumerate(report_verify._acquisitions_columns())},
               first_data_row=3, last_data_row=3)
    sub = dict(sheet_name='Disposals', col_index={c['key']: i for i, c in enumerate(report_verify._disposals_columns())},
               first_data_row=3, last_data_row=3)
    columns = {key: [{k: v for k, v in col.items() if k != 'formula'} for col in fun()]
               for key, fun in [('acquisitions', report_verify._acquisitions_columns),
                                ('disposals', report_verify._disposals_columns),
                                ('attachments', report_verify._evidence_columns),
                                ('quarantines', report_verify._quarantined_columns),
                                ('asset_rows', lambda: report_verify._control_columns(ref, sub))]}
    result = dict(formats=book.document['formats'], styles={key: book.formats[style] for key, style in styles.items()},
                  columns=columns, readme={country: report_verify._verify_readme_rows(gains_algorithm='__METHOD__',
                      tax_country=country, fiat_currency='__FIAT__', wallet_scope_label='__WALLET__') for country in ('', 'at')})
    book.close()
    return result


def build(root):
    cases = [c for c in json.loads((ROOT / 'crates/reports/tests/data/artifact_csv.json').read_text())
             if c['command'] == 'reports.export-csv']
    extra = [
        ('no-verify', {}, '', False),
        ('wallet-verification', {'wallet': 'hot', 'history_limit': 1}, '', True),
        ('austrian', {}, "UPDATE profiles SET tax_country='at',gains_algorithm='MOVING_AVERAGE_AT';", True),
        ('hifo', {}, "UPDATE profiles SET gains_algorithm='HIFO';", True),
        ('acquisition-fee', {}, "UPDATE journal_entries SET entry_type='acquisition',quantity=1001 WHERE id='first'; UPDATE journal_entries SET entry_type='transfer_fee' WHERE id='fee';", True),
        ('gain-mismatch', {}, "UPDATE journal_entries SET gain_loss=99.31 WHERE id='z';", True),
        ('large-msat', {}, "UPDATE journal_entries SET quantity=-9007199254740993 WHERE id='z';", True),
        ('quality', {}, "UPDATE journal_entries SET pricing_quality='coarse_fallback',pricing_source_kind='manual',description='日本語 € 😀';", True),
        ('non-taxable', {}, "UPDATE transactions SET taxability_override=0; UPDATE journal_entries SET at_category='neu_swap' WHERE id='z';", True),
        ('quarantine-json', {}, "UPDATE journal_quarantines SET detail_json='{\"nested\":[true,null,1],\"empty\":{}}';", True),
        ('quarantine-raw', {}, "UPDATE journal_quarantines SET detail_json='not-json';", True),
        ('missing-rate-source', {}, "UPDATE rates_cache SET source='';", True),
        ('all-ledger-types', {}, "UPDATE journal_entries SET entry_type='transfer_in' WHERE id='income'; UPDATE journal_entries SET entry_type='transfer_out' WHERE id='z';", True),
    ]
    cases += [dict(name=name, args_json=json.dumps({**args, 'verify': verify}), sql=sql) for name, args, sql, verify in extra]
    result = []
    for case in cases:
        with tempfile.TemporaryDirectory(dir=root, prefix='full-xlsx-') as path:
            path = Path(path)
            conn = open_db(path / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + case['sql'])
            args = json.loads(case['args_json'])
            captured = []
            class Capture(RecordingWorkbook):
                def __init__(self, *args, **kwargs):
                    super().__init__(*args, **kwargs)
                    captured.append(self)
            try:
                with patch('xlsxwriter.Workbook', Capture), patch.object(reports, 'datetime', FixedDateTime), patch.object(reports, '_register_saved_export', return_value=None):
                    reports.export_xlsx_report(conn, args.get('workspace'), args.get('profile'), path / 'report.xlsx',
                        replace(_report_hooks(), now_iso=lambda: STAMP), wallet_ref=args.get('wallet'),
                        history_limit=args.get('history_limit'), verify=args.get('verify', True))
                expected = dict(document=captured[0].document)
            except AppError as e:
                expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as e:
                expected = dict(error=dict(code='internal_error', message=str(e), hint=None, details=None, retryable=False))
            result.append(dict(name=case['name'], sql=case['sql'], args_json=case['args_json'], expected=expected))
            conn.close()
    return result


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='full-xlsx-generator-') as root:
        root = Path(root)
        for dest, data in [(META, metadata(root)), (OUT, build(root))]:
            content = ('[\n' + ',\n'.join(json.dumps(case, ensure_ascii=True, separators=(',', ':')) for case in data) + '\n]\n') if dest == OUT else json.dumps(data, ensure_ascii=True, indent=2) + '\n'
            if '--check' in sys.argv:
                assert dest.read_text() == content, f'{dest.name} drifted'
            else:
                dest.write_text(content)
