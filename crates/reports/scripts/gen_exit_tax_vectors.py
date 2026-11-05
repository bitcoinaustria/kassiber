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
         '2025-01-01T00:00:00.123Z', ' 2025-02-01 ', '', None, 'bad', '2025-02-30', 20250101, 0,
         '2025-01-01T12:00:00', '2025-01-01 12:00:00', '2025-01-01T12:00:00.123456',
         '2025-01-01T12', '20250101T120000', '2025-01-01Z12:00:00',
         '2025-01-01T12:00:00+01:00', '20250101T120000+0100',
         '2025-01-01T12:00:00Z', '2025-01-01T12:00:00-05:30',
         '2024-03-31Z', 'garbage', '1711843200', 'T24:00'])]
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
    # Round 6/6b/6c/6d boundaries use explicit civil days and source instants.
    clear = "DELETE FROM rates_cache; UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL;"
    def cache(at, rate=65000, source='manual'):
        return f"INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('BTC-EUR','{at}',{rate},'{source}','fixed');"
    for country in ('at', 'generic'):
        for method in ('fifo','lifo','hifo','lofo','moving_average') + (('moving_average_at',) if country == 'at' else ()):
            result.append((f'method-{country}-{method}', {'departure_date':'2024-12-31'}, clear + f"UPDATE profiles SET tax_country='{country}',gains_algorithm=' {method.upper()} ';" + cache('2024-12-31T12:00:00Z')))
    for day, cut in [('2024-01-15','2024-01-15T23:00:00Z'),('2024-07-15','2024-07-15T22:00:00Z'),('2024-03-31','2024-03-31T22:00:00Z'),('2024-10-27','2024-10-27T23:00:00Z')]:
        from datetime import timedelta
        instant = datetime.fromisoformat(cut.replace('Z','+00:00'))
        for delta in (-1,0,1):
            at = (instant + timedelta(microseconds=delta)).isoformat().replace('+00:00','Z')
            result.append((f'vienna-{day}-{delta}', {'departure_date':day}, clear + f"UPDATE journal_entries SET occurred_at='{at}' WHERE id='new';" + cache(at)))
        for source in ('cache','transaction'):
            for older in (0,1):
                at = (instant - timedelta(hours=24,microseconds=older)).isoformat().replace('+00:00','Z')
                sql = cache(at) if source == 'cache' else f"UPDATE transactions SET occurred_at='{at}',fiat_rate=70000 WHERE id='in';"
                result.append((f'vienna-age-{day}-{source}-{older}', {'departure_date':day},clear+sql))
    for departure in ('2024-03-31T22:30:00Z','2024-03-31T23:30:00-01:00','2024-10-27T23:30:00Z'):
        result.append(('civil-input-'+departure, {'departure_date':departure},clear))
    # Main sync 1 (#706): the 27.5 % rate and KZ 174/176 start with departures
    # on 1 March 2022, by the departure day as the owner names it.
    for departure in ('2022-02-28','2022-03-01','2022-02-28T22:30:00Z','2022-02-28T23:30:00Z','2022-02-28T23:30:00+01:00'):
        for country in ('at','generic'):
            result.append((f'old-rules-{country}-{departure}', {'departure_date':departure}, f"UPDATE profiles SET tax_country='{country}';"))
    for source in ('cache','transaction'):
        for label, at in [('24h','2024-12-30T23:00:00Z'),('too-old','2024-12-30T22:59:59.999999Z'),('inside','2024-12-31T22:59:59.999999Z'),('cutoff','2024-12-31T23:00:00Z'),('future','2024-12-31T23:00:00.000001Z'),('offset','2025-01-01T00:59:59.123456+02:00')]:
            sql = cache(at) if source == 'cache' else f"UPDATE transactions SET occurred_at='{at}',fiat_rate=70000 WHERE id='in';"
            result.append((f'quote-{source}-{label}', {'departure_date':'2024-12-31'}, clear + sql))
    for name, sql in [
        ('zero-cache',cache('2024-12-31T13:00:00Z',0)),
        ('zero-transaction',"UPDATE transactions SET occurred_at='2024-12-31T13:00:00Z',fiat_rate=0 WHERE id='in';"),
        ('closest-transaction',cache('2024-01-01T00:00:00Z',1)+"UPDATE transactions SET occurred_at='2024-12-31T12:00:00Z',fiat_rate=70000 WHERE id='in';"),
        ('closest-cache',cache('2024-12-31T13:00:00Z')+"UPDATE transactions SET occurred_at='2024-12-31T12:00:00Z',fiat_rate=70000 WHERE id='in';"),
        ('equal-cache-priority',cache('2024-12-31T13:00:00Z',66000,'provider')+cache('2024-12-31T14:00:00+01:00')+"UPDATE transactions SET occurred_at='2024-12-31T13:00:00Z',fiat_rate=70000 WHERE id='in';"),
        ('cache-instant-order',cache('2024-12-31T22:30:00+02:00',61000)+cache('2024-12-31T21:00:00Z',62000)),
        ('transaction-instant-order',"UPDATE transactions SET occurred_at='2024-12-31T21:00:00Z',fiat_rate=62000 WHERE id='in'; UPDATE transactions SET occurred_at='2024-12-31T22:30:00+02:00',fiat_rate=61000 WHERE id='out';"),
        ('transaction-cutoff-does-not-mask-earlier',"UPDATE transactions SET occurred_at='2024-12-31T22:59:59.999999Z',fiat_rate=62000 WHERE id='in'; UPDATE transactions SET occurred_at='2024-12-31T23:00:00Z',fiat_rate=61000 WHERE id='out';"),
        ('cache-cutoff-does-not-mask-earlier',cache('2024-12-31T22:59:59.999999Z',62000)+cache('2024-12-31T23:00:00Z',61000)),
        ('wallet-transfer-only',"UPDATE journal_entries SET entry_type='transfer_in',asset='OTHER' WHERE id='new';"),
        ('wallet-transfer-priced',"UPDATE journal_entries SET entry_type='transfer_in',asset='OTHER' WHERE id='new'; INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at) VALUES('OTHER-EUR','2024-12-31T12:00:00Z',12,'manual','fixed');"),
        ('current-holdings-ignored',"UPDATE journal_wallet_holdings SET quantity=999999999999;"),
        ('wallet-transfer-fee',"UPDATE journal_entries SET entry_type='transfer_out',quantity=-150000000000,occurred_at='2024-12-31T12:00:00Z' WHERE id='z'; UPDATE journal_entries SET entry_type='transfer_fee',quantity=-50000000000,occurred_at='2024-12-31T12:00:00Z' WHERE id='fee';"),
        ('outflow-marker-override',"UPDATE journal_entries SET description='at_regime=alt',at_category='neu_gain',occurred_at='2024-12-31T12:00:00Z' WHERE id='z';"),
        ('legacy-outflow-category',"UPDATE journal_entries SET quantity=-100000000000,cost_basis=8000,at_category='alt_taxfree',occurred_at='2024-12-31T12:00:00Z' WHERE id='z';"),
        ('bad-journal-instant',"UPDATE journal_entries SET occurred_at='bad' WHERE id='new';"),
        ('missing-journal-instant',"UPDATE journal_entries SET occurred_at='' WHERE id='new';"),
        ('old-semantics',"UPDATE profiles SET required_journal_semantics_version=2,journal_semantics_version=1;"),
        ('unstamped',"UPDATE profiles SET required_journal_semantics_version=2,journal_semantics_version=0;"),
    ]:
        result.append((name,{'departure_date':'2024-12-31'},clear+sql))
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
    meta['missing'] = ('EXIT-005: No usable cached or transaction quote found for one or more assets — run '
        '`kassiber rates sync`; market value, gain, and estimated tax stay '
        'incomplete (null) until those holdings are priced.')
    with tempfile.TemporaryDirectory(dir=scratch) as root:
        for dest, data in [(META, meta), (OUT, build(root))]:
            content = ('[\n' + ',\n'.join(json.dumps(c, ensure_ascii=True, separators=(',', ':')) for c in data) + '\n]\n') if dest == OUT else json.dumps(data, ensure_ascii=True, indent=2) + '\n'
            if '--check' in sys.argv:
                assert dest.read_text() == content, f'{dest.name} drifted'
            else:
                dest.write_text(content)
