import json
from dataclasses import asdict, replace
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import year_handoff as handoff
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, STAMP

BASE = 'DELETE FROM journal_quarantines;'
OUT = ROOT / 'crates/reports/tests/data/year_handoff.json'
META = ROOT / 'crates/reports/src/year_handoff_metadata.json'


def cases():
    result = [(f'year-{year}', {'year': year}, '') for year in (2023, 2024, 2025, 2026)]
    result += [(f'argument-{i}', {'year': value}, '') for i, value in enumerate(
        [None, True, False, '２０２４', '2_024', ' 2024 ', 2024.9, 2008, 2101, [], {}, 'bad'])]
    changes = [
        ('neutral-only', "DELETE FROM journal_entries WHERE id!='swap';"),
        ('amendment-history', "INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,created_at) VALUES('old-handoff','ws','p','year-handoff.xlsx','filed',2024,2024,printf('%064d',1),'fixed'); INSERT INTO custody_filed_report_impacts(id,workspace_id,profile_id,filed_report_snapshot_id,component_id,review_id,gap_id,affected_period_start_year,affected_period_end_year,amendment_warning,created_at) VALUES('old-impact','ws','p','old-handoff','component','review','gap',2024,2024,'Review amendment','fixed');"),
        ('inbound-fee', "UPDATE transactions SET amount_includes_fee=1 WHERE id='in';"),
        ('external-pair-id', "UPDATE transactions SET external_id='EXTERNAL-RECEIVE' WHERE id='in'; UPDATE journal_custody_economic_relations SET target_transaction_id='in';"),
        ('pair-multiple', "UPDATE journal_custody_economic_relations SET relation_id='abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789',target_transaction_id='in';"),
        ('income-in-year', "UPDATE journal_entries SET occurred_at='2024-02-01T00:00:00Z' WHERE id='income';"),
        ('fee-only', "DELETE FROM journal_entries WHERE id!='fee';"),
        ('non-taxable', "UPDATE transactions SET taxability_override=0;"),
        ('empty-book', "DELETE FROM journal_entries; DELETE FROM transactions; UPDATE profiles SET last_processed_tx_count=0;"),
        ('quarantine-prior', "UPDATE transactions SET occurred_at='2023-07-01T00:00:00Z' WHERE id='in'; INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('in','ws','p','missing_price','fixed');"),
        ('reconciles', "DELETE FROM journal_entries WHERE id NOT IN ('first','z','fee'); UPDATE journal_entries SET transaction_id='in',entry_type='acquisition',quantity=123456789001,occurred_at='2024-01-01T00:00:00Z',fiat_value=1234,cost_basis=NULL WHERE id='first'; UPDATE journal_entries SET quantity=-123456789 WHERE id='z';"),
        ('pooled-opening', "UPDATE journal_entries SET entry_type='acquisition',quantity=100000000000,fiat_value=12345,occurred_at='2023-01-01T00:00:00Z' WHERE id='first'; UPDATE journal_entries SET entry_type='transfer_out',quantity=-30000000000,cost_basis=NULL WHERE id='a'; UPDATE journal_entries SET entry_type='transfer_in',quantity=30000000000,fiat_value=0,wallet_id='hot' WHERE id='z';"),
        ('cache-manual-tie', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-12-31T23:59:59Z',88888,'88888','market','z'),('BTC-EUR','2024-12-31T23:59:59Z',77777,'77777','manual','a');"),
        ('cache-fetched-tie', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-12-31T23:59:59Z',88888,'88888','market','z'),('BTC-EUR','2024-12-31T23:59:59Z',77777,'77777','other','a');"),
        ('austrian', "UPDATE profiles SET tax_country='at';"),
        ('austrian-spaces', "UPDATE profiles SET tax_country=' at ';"),
        ('hifo-usd', "UPDATE profiles SET gains_algorithm='HIFO',fiat_currency='USD';"),
        ('stale', 'UPDATE profiles SET last_processed_input_version=-1;'),
        ('freshness', "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed');"),
        ('quarantine-before', "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('in','ws','p','missing_price','fixed');"),
        ('quarantine-after', "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('earn','ws','p','missing_price','fixed');"),
        ('acquisition', "UPDATE journal_entries SET entry_type='acquisition',quantity=123456789001,fiat_value=15000 WHERE id='first';"),
        ('income-acquisition', "UPDATE journal_entries SET entry_type='acquisition',quantity=123456789001,fiat_value=15000 WHERE id='first'; UPDATE journal_entries SET entry_type='income',quantity=123456789001 WHERE id='a';"),
        ('transfers', "UPDATE journal_entries SET entry_type='transfer_out' WHERE id='a'; UPDATE journal_entries SET entry_type='transfer_in',quantity=23456788 WHERE id='z'; UPDATE journal_entries SET entry_type='transfer_fee' WHERE id='fee';"),
        ('timing', "UPDATE journal_entries SET occurred_at='2023-12-31T23:59:59Z' WHERE id='first';"),
        ('future-classification', "UPDATE journal_entries SET occurred_at='2025-01-02T00:00:00Z' WHERE id='z';"),
        ('unknown-entry', "UPDATE journal_entries SET entry_type='mystery' WHERE id='z';"),
        ('exact-prices', "UPDATE journal_entries SET fiat_value_exact='123.4567890123456789012345678901',cost_basis_exact='100.000000000000000001',proceeds_exact='129.000000000000000001',gain_loss_exact='29.000000000000000001';"),
        ('large-msat', "UPDATE journal_entries SET quantity=-9007199254740993 WHERE id='z'; UPDATE transactions SET amount=9007199254740993 WHERE id='out';"),
        ('debit-includes-fee', "UPDATE transactions SET amount_includes_fee=1 WHERE id='out';"),
        ('zero-quantity', 'UPDATE journal_entries SET quantity=0;'),
        ('only-income', "DELETE FROM journal_entries WHERE entry_type!='income';"),
        ('missing-rates', 'UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL; DELETE FROM rates_cache;'),
        ('fallback-value', "UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=1234,fiat_value_exact='1234.567890123456789';"),
        ('empty-exact', "UPDATE transactions SET fiat_rate_exact='',fiat_value_exact=''; UPDATE journal_entries SET cost_basis_exact='',fiat_value_exact='';"),
        ('excluded', "UPDATE transactions SET occurred_at='2024-07-01T00:00:00Z' WHERE id='excluded';"),
        ('markers', "UPDATE journal_entries SET description=' Some　words at_regime=neu at_custom=one tail ';"),
        ('asset-case', "UPDATE transactions SET asset='btc' WHERE id='out'; UPDATE journal_entries SET asset='btc' WHERE id='z';"),
        ('liquid-case', "UPDATE transactions SET asset=printf('%064X',255) WHERE id='out'; UPDATE journal_entries SET asset=printf('%064x',255) WHERE id='z';"),
        ('external-asset', "UPDATE transactions SET asset='ext:token:usdt' WHERE id='out'; UPDATE journal_entries SET asset='ext:token:usdt' WHERE id='z';"),
        ('labels', "UPDATE wallets SET label='日本語 € 😀' WHERE id='cold'; UPDATE transactions SET note='=HYPERLINK(\"https://example.invalid\")';"),
        ('deprecated', "UPDATE wallets SET config_json='{\"deprecated\":true,\"last_synced_at\":\"2026-01-01T00:00:00Z\"}' WHERE id='cold';"),
        ('invalid-date', "UPDATE journal_entries SET occurred_at='bad' WHERE id='z';"),
        ('offset-date', "UPDATE journal_entries SET occurred_at='2025-01-01T00:30:00+02:00' WHERE id='z';"),
        ('cache', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-12-31T23:59:59Z',77777,'77777.123456789','manual','fixed');"),
        ('cache-exclusive', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2025-01-01T00:00:00Z',77777,'77777','manual','fixed');"),
        ('cache-offset', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2025-01-01T00:30:00+02:00',77777,'77777','manual','fixed');"),
        ('cache-zero', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-12-31T23:59:59Z',0,'0','','fixed');"),
        ('cache-fraction-after', "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR','2024-12-31T23:59:59.5Z',77777,'77777','manual','fixed');"),
    ]
    result += [(name, {'year': 2024}, sql) for name, sql in changes]
    # Main sync 1 (#706): a crypto Kennzahl before 1 March 2022 (Vienna) is
    # stated in no field; KZ 801 is unaffected.
    early = ("UPDATE journal_entries SET occurred_at='2022-02-28T22:59:59Z' WHERE id='a'; "
             "UPDATE journal_entries SET occurred_at='2022-02-28T23:00:00Z',at_category='neu_gain',at_kennzahl=174 WHERE id='z'; "
             "UPDATE journal_entries SET occurred_at='2022-02-10T00:00:00Z',at_category='alt_spekulation',at_kennzahl=801 WHERE id='first'; "
             "UPDATE journal_entries SET occurred_at='2022-01-05T00:00:00Z' WHERE id='income'; "
             "UPDATE journal_entries SET at_kennzahl='' WHERE id='fee';")
    for country in ('at', 'generic'):
        result.append((f'kennzahl-2022-{country}', {'year': 2022}, f"UPDATE profiles SET tax_country='{country}'; " + early))
    return result


def build(root):
    result = []
    metadata = None
    for name, args, sql in cases():
        with tempfile.TemporaryDirectory(dir=root) as raw:
            path = Path(raw)
            conn = open_db(path / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + BASE + sql)
            captured = []
            class Capture(RecordingWorkbook):
                def __init__(self, *args, **kwargs):
                    super().__init__(*args, **kwargs)
                    captured.append(self)
            try:
                hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
                with patch('xlsxwriter.Workbook', Capture), patch.object(handoff, '_register_saved_export', return_value=None):
                    payload = handoff.export_year_handoff_xlsx(conn, None, None, path / 'handoff.xlsx', hooks, year=args.get('year'))
                expected = dict(document=captured[0].document, metadata={k:v for k,v in payload.items() if k not in {"file", "bytes", "report_snapshot"}})
                if metadata is None:
                    report = handoff.build_year_handoff(conn, None, None, hooks, year=2024)
                    sheets = handoff._handoff_sheets(report)
                    metadata = dict(sheets=[dict(name=s['name'], columns=[asdict(c) for c in s['columns']]) for s in sheets],
                        notes=[r['value'] for r in handoff._overview_rows(report) if r['group'] == 'Notes'])
            except AppError as e:
                expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as e:
                expected = dict(error=dict(code='internal_error', message=str(e), hint=None, details=None, retryable=False))
            result.append(dict(name=name, sql=BASE+sql, args_json=json.dumps(args), expected=expected))
            conn.close()
    return result, metadata


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(dir=ROOT / 'target/scratch') as raw:
        cases, metadata = build(Path(raw))
        for dest, data in [(OUT, cases), (META, metadata)]:
            content = ('[\n' + ',\n'.join(json.dumps(case, ensure_ascii=True, separators=(',', ':')) for case in data) + '\n]\n') if dest == OUT else json.dumps(data, ensure_ascii=True, indent=2) + '\n'
            if '--check' in sys.argv:
                assert dest.read_text() == content, f'{dest.name} drifted'
            else:
                dest.write_text(content)
