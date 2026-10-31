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
from kassiber.core import exit_tax, reports
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, STAMP

OUT = ROOT / 'crates/reports/tests/data/exit_tax.json'
META = ROOT / 'crates/reports/src/exit_tax_metadata.json'
BASE = """
UPDATE profiles SET tax_country='at',gains_algorithm='MOVING_AVERAGE_AT';
UPDATE journal_entries SET entry_type='acquisition',quantity=100000000000,fiat_value=5000,cost_basis=NULL,occurred_at='2020-01-01T00:00:00Z',at_category=NULL WHERE id='first';
INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,entry_type,asset,quantity,fiat_value,created_at)
VALUES('new','ws','p','in','cold','acct','2024-01-01T00:00:00Z','acquisition','BTC',200000000000,20000,'fixed');
"""

class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc).astimezone(tz)


def cases():
    result = [('baseline', {}, '')]
    result += [(f'destination-{i}', {'destination': value}, '') for i, value in enumerate(
        ['third_country', 'eu', 'EEA', ' eU-eEa ', 'euea', 'third', 'non-eu', 'noneu', '', None, 'bad', 1])]
    result += [(f'departure-{i}', {'departure_date': value}, '') for i, value in enumerate(
        ['2020-01-01', '2024-12-31', '2025-01-01T00:30:00+02:00', '20240101', '2025-W01-1',
         '2025-01-01T00:00:00.123Z', ' 2025-02-01 ', '', None, 'bad', '2025-02-30', 20250101, 0])]
    changes = [
        ('generic', "UPDATE profiles SET tax_country='generic',gains_algorithm='HIFO',fiat_currency='USD';"),
        ('generic-invalid-days', "UPDATE profiles SET tax_country='generic',tax_long_term_days=-1;"),
        ('generic-invalid-text-days', "UPDATE profiles SET tax_country='generic',tax_long_term_days='not-a-day';"),
        ('generic-negative-real-days', "UPDATE profiles SET tax_country='generic',tax_long_term_days=-1.5;"),
        ('generic-subday-negative', "UPDATE profiles SET tax_country='generic',tax_long_term_days=-0.5;"),
        ('generic-space-days', "UPDATE profiles SET tax_country='generic',tax_long_term_days=' ';"),
        ('generic-empty-days', "UPDATE profiles SET tax_country='generic',tax_long_term_days='';"),
        ('generic-unicode-days', "UPDATE profiles SET tax_country='generic',tax_long_term_days='１２';"),
        ('at-usd', "UPDATE profiles SET fiat_currency='USD';"),
        ('at-hifo', "UPDATE profiles SET gains_algorithm='HIFO';"),
        ('no-entries', 'DELETE FROM journal_entries;'),
        ('only-income', "DELETE FROM journal_entries WHERE entry_type!='income';"),
        ('transfers', "UPDATE journal_entries SET entry_type='transfer_in' WHERE id='new'; UPDATE journal_entries SET entry_type='transfer_out' WHERE id='first';"),
        ('category-over-date', "UPDATE journal_entries SET at_category='alt_taxfree' WHERE id='new';"),
        ('marker-first', "UPDATE journal_entries SET description='at_regime=alt at_regime=neu',at_category='neu_gain' WHERE id='new';"),
        ('invalid-first-marker', "UPDATE journal_entries SET description='at_regime=bad at_regime=alt',at_category='neu_gain' WHERE id='new';"),
        ('unicode-marker', "UPDATE journal_entries SET description='other　at_regime=alt' WHERE id='new';"),
        ('neu-marker', "UPDATE journal_entries SET description='at_regime=neu',at_category='alt_taxfree' WHERE id='first';"),
        ('cutoff-before', "UPDATE journal_entries SET occurred_at='2021-02-28T22:59:59Z' WHERE id='new';"),
        ('cutoff-at', "UPDATE journal_entries SET occurred_at='2021-02-28T23:00:00Z' WHERE id='new';"),
        ('cutoff-offset', "UPDATE journal_entries SET occurred_at='2021-03-01T00:00:00+01:00' WHERE id='new';"),
        ('negative-basis', "UPDATE journal_entries SET cost_basis=-100 WHERE id='new';"),
        ('outflow-overdraw', "UPDATE journal_entries SET quantity=-300000000000,cost_basis=40000 WHERE id='z';"),
        ('loss', "UPDATE journal_entries SET fiat_value=900000 WHERE id='new';"),
        ('basis-precedence', "UPDATE journal_entries SET cost_basis=111,fiat_value=999 WHERE id='new';"),
        ('invalid-unused-price', "UPDATE transactions SET fiat_value_exact='invalid';"),
        ('invalid-unused-ledger', "UPDATE journal_entries SET proceeds_exact='invalid' WHERE id='new';"),
        ('exact-basis', "UPDATE journal_entries SET cost_basis=1,cost_basis_exact='123.455',fiat_value_exact='999' WHERE id='new';"),
        ('half-sat', "UPDATE journal_entries SET quantity=1500,fiat_value_exact='0.005' WHERE id='new';"),
        ('rounding-ties', "DELETE FROM journal_entries WHERE quantity<0; UPDATE journal_entries SET quantity=2500,fiat_value_exact='0.005' WHERE id='new';"),
        ('large-msat', "UPDATE journal_entries SET quantity=9007199254740993 WHERE id='new';"),
        ('different-assets', "UPDATE journal_entries SET asset='OTHER' WHERE id='new';"),
        ('missing-rates', 'DELETE FROM rates_cache; UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL;'),
        ('no-rate-alt', "UPDATE journal_entries SET asset='OTHER' WHERE id='first';"),
        ('future-cache', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2030-01-01T00:00:00Z',999,'999','manual','fixed');"),
        ('manual-cache', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2025-01-01T00:00:00Z',60000,'60000.125','provider','z'),('BTC-EUR','2025-01-01T00:00:00Z',70000,'70000.375','manual','a');"),
        ('fraction-cache', "INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','2026-01-02T23:59:59.000001Z',123,'manual','fixed');"),
        ('offset-cache', "INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','2026-01-03T00:30:00+02:00',123,'manual','fixed');"),
        ('nonpositive-cache', "INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','2025-01-01T00:00:00Z',0,'manual','a');"),
        ('nonpositive-fallback', 'DELETE FROM rates_cache; UPDATE transactions SET fiat_rate=0,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL;'),
        ('fiat-value-fallback', 'DELETE FROM rates_cache; UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=99,fiat_value_exact=NULL;'),
        ('empty-exact-fallback-rate', "DELETE FROM rates_cache; UPDATE transactions SET fiat_rate=999,fiat_rate_exact='',fiat_value=99,fiat_value_exact=NULL;"),
        ('empty-exact-fallback-value', "DELETE FROM rates_cache; UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=99,fiat_value_exact='';"),
        ('no-quarantine', 'DELETE FROM journal_quarantines;'),
        ('all-tagged', "UPDATE journal_entries SET description='at_regime=alt' WHERE quantity<0;"),
        ('two-loss-assets', "UPDATE journal_entries SET asset='LBTC',fiat_value=1 WHERE id='first'; UPDATE journal_entries SET fiat_value=900000 WHERE id='new';"),
        ('no-wallet-holdings', 'DELETE FROM journal_wallet_holdings;'),
        ('amendment-history', """INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,created_at)
            VALUES('old-exit','ws','p','exit-tax.xlsx','filed',2024,2024,printf('%064d',1),'fixed');
            INSERT INTO custody_filed_report_impacts(id,workspace_id,profile_id,filed_report_snapshot_id,component_id,review_id,gap_id,affected_period_start_year,affected_period_end_year,amendment_warning,created_at)
            VALUES('old-impact','ws','p','old-exit','component','review','gap',2024,2024,'Review an amendment before relying on this filed report.','fixed');"""),
        ('unpriced-context-wallet', "UPDATE journal_wallet_holdings SET asset='OTHER';"),
        ('unicode', "UPDATE profiles SET label='日本語 € 😀' WHERE id='p'; UPDATE wallets SET label='ä Ω' WHERE id='cold';"),
    ]
    result += [(name, {}, sql) for name, sql in changes]
    result.append(('entry-offset-lexical', {'departure_date': '2023-12-31'}, "UPDATE journal_entries SET occurred_at='2024-01-01T00:30:00+02:00' WHERE id='new';"))
    result.append(('invalid-ledger-before-date', {'departure_date': 'bad'}, "UPDATE journal_entries SET proceeds_exact='invalid' WHERE id='new';"))
    inherited = json.loads((ROOT / 'crates/reports/tests/data/full_xlsx.json').read_text())
    result += [('gate-' + c['name'], json.loads(c['args_json']), c['sql']) for c in inherited if 'error' in c['expected'] and 'history' not in c['name'] and 'wallet' not in c['name']]
    return result


def build(root):
    result = []
    for name, args, sql in cases():
        with tempfile.TemporaryDirectory(dir=root) as path:
            path = Path(path)
            conn = open_db(path / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + BASE + sql)
            captured = []
            class Capture(RecordingWorkbook):
                def __init__(self, *args, **kwargs):
                    super().__init__(*args, **kwargs)
                    captured.append(self)
            try:
                with patch('xlsxwriter.Workbook', Capture), patch.object(exit_tax, 'datetime', FixedDateTime):
                    report = reports.report_exit_tax(conn, args.get('workspace'), args.get('profile'), _report_hooks(),
                        departure_date=args.get('departure_date'), destination=args.get('destination'))
                    reports.export_exit_tax_xlsx_report(conn, args.get('workspace'), args.get('profile'), path / 'report.xlsx',
                        replace(_report_hooks(), now_iso=lambda: STAMP), departure_date=args.get('departure_date'), destination=args.get('destination'))
                expected = dict(report=report, text='\n'.join(exit_tax.format_exit_tax_lines(report)), document=captured[0].document)
            except AppError as e:
                expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as e:
                expected = dict(error=dict(code='internal_error', message=str(e), hint=None, details=None, retryable=False))
            result.append(dict(name=name, sql=BASE + sql, args_json=json.dumps(args), expected=expected))
            conn.close()
    return result


if __name__ == '__main__':
    scratch = ROOT / 'target/r5-reports14/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    meta = {k: v for k, v in vars(exit_tax).items() if k.startswith('_ASSUMPTION_') or k == 'EXIT_TAX_REVIEW_GATE'}
    meta['missing'] = ('EXIT-005: No cached rate found for one or more assets — run '
        '`kassiber rates sync`; market value, gain, and estimated tax stay '
        'incomplete (null) until those holdings are priced.')
    with tempfile.TemporaryDirectory(dir=scratch) as root:
        for dest, data in [(META, meta), (OUT, build(root))]:
            content = ('[\n' + ',\n'.join(json.dumps(c, ensure_ascii=True, separators=(',', ':')) for c in data) + '\n]\n') if dest == OUT else json.dumps(data, ensure_ascii=True, indent=2) + '\n'
            if '--check' in sys.argv:
                assert dest.read_text() == content, f'{dest.name} drifted'
            else:
                dest.write_text(content)
