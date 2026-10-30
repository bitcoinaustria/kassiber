#!/usr/bin/env python3
"""Freeze artifact bytes from Python without publishing or registering a report."""
from dataclasses import replace
from datetime import datetime, timezone
import csv
import io
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.db import open_db
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'crates/reports/tests/data/artifact_csv.json'
STAMP = '2026-01-02T03:04:05Z'


class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc).astimezone(tz)


def encode(rows):
    output = io.StringIO(newline='')
    csv.writer(output).writerows(rows)
    return output.getvalue()


def build():
    inherited = json.loads((OUT.parent / 'vectors.json').read_text())
    specs = []
    for case in inherited:
        if case['group'] in {'summary', 'holdings', 'readiness'}:
            specs.append((case['name'], 'reports.export-csv', case['sql'], case['args']))
        if case['group'] in {'austrian', 'austrian-text'}:
            specs.append((case['name'], 'reports.export-austrian-e1kv-csv', case['sql'], case['args']))
    specs += [('history-limit-' + str(n), 'reports.export-csv', '', {'history_limit': n})
              for n in (-1, 0, 1, 100, None, True, False, 1.9, -1.9, ' １ ', '1_0',
                        '', [], {}, 10**80, float('nan'), float('inf'))]
    specs += [('empty', 'reports.export-csv',
               'DELETE FROM journal_entries; DELETE FROM journal_wallet_holdings; '
               'DELETE FROM journal_account_holdings; DELETE FROM transactions; '
               'UPDATE profiles SET last_processed_tx_count=0 WHERE id=\'p\';', {})]
    for name, sql in [
        ('price-zero', "UPDATE transactions SET fiat_rate=0,fiat_rate_exact='0' WHERE id='out';"),
        ('price-empty', "UPDATE transactions SET fiat_rate_exact='',fiat_value_exact='' WHERE id='out';"),
        ('price-cash', "UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value_exact='5.12345678901234567890123456' WHERE id='out';"),
        ('fee-only', "UPDATE transactions SET amount=0,fee=1001,fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value_exact='1.12345678901234567890123456' WHERE id='out';"),
        ('no-price', "UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL;"),
        ('csv-quotes', "UPDATE transactions SET description='Comma, quote \" CR'||char(13)||'LF'||char(10)||'€',note='Line'||char(10)||'note',counterparty='Party, Inc.' WHERE id='out';"),
        ('labels', "UPDATE wallets SET label='Cold, \"€\"' WHERE id='cold';"),
        ('invalid-descriptor', "UPDATE wallets SET config_json='{\"descriptor\":\"invalid\",\"backend\":\"local\"}' WHERE id='cold';"),
        ('descriptor-network-mismatch', "UPDATE wallets SET config_json='{\"descriptor\":\"wpkh(tpub123/0/*)\",\"network\":\"main\"}' WHERE id='cold';"),
        ('descriptor-network-mix', "UPDATE wallets SET config_json='{\"descriptor\":\"wpkh(xpub123/0/*)\",\"change_descriptor\":\"wpkh(tpub123/1/*)\"}' WHERE id='cold';"),
        ('live-chain', "UPDATE wallets SET config_json='{\"chain\":\"bitcoin\",\"network\":\"mainnet\",\"gap_limit\":17}' WHERE id='cold';"),
        ('empty-config', "UPDATE wallets SET config_json='' WHERE id='cold';"),
        ('null-config', "UPDATE wallets SET config_json='null' WHERE id='cold';"),
        ('array-config', "UPDATE wallets SET config_json='[]' WHERE id='cold';"),
        ('bad-config', "UPDATE wallets SET config_json='{' WHERE id='cold';"),
        ('float-quantity', "UPDATE journal_wallet_holdings SET quantity=100000000000000001 WHERE id='wz';"),
        ('taxability', "UPDATE transactions SET taxability_override=0 WHERE id='out';"),
        ('swap-nontaxable', "UPDATE journal_entries SET at_category='neu_swap' WHERE transaction_id='out';"),
    ]:
        specs.append((name, 'reports.export-csv', sql, {}))
    specs += [
        ('scoped-null-other-wallet', 'reports.export-csv',
         "UPDATE wallets SET config_json='null' WHERE id='hot';", {'wallet': 'Cold'}),
        ('scoped-typed-other-wallet', 'reports.export-csv',
         "UPDATE wallets SET config_json='{\"descriptor\":\"wpkh(tpub123/0/*)\",\"network\":\"main\"}' WHERE id='hot';",
         {'wallet': 'Cold'}),
        ('inventory-error-before-pricing', 'reports.export-csv',
         "UPDATE wallets SET config_json='{\"descriptor\":\"wpkh(tpub123/0/*)\",\"network\":\"main\"}' WHERE id='cold'; "
         "UPDATE transactions SET fiat_rate_exact='invalid' WHERE id='no-tax';", {}),
    ]
    # Tags and literal links are presentation, never acquisition permission.
    specs.append(('evidence', 'reports.export-csv', """
        INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('tag-z','ws','p','z','Z','fixed'),('tag-a','ws','p','a','A','fixed');
        INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('out','tag-z'),('out','tag-a');
        INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,source_url,original_filename,created_at)
          VALUES('url','ws','p','out','url',' Tax, receipt ',' https://example.invalid/a?x=1 ','','a'),
                ('file','ws','p','out','file','ignored','','invoice,€\".pdf','b');
    """, {}))
    transfer_sql = """
        INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,transaction_id,entry_type,asset,quantity,occurred_at,description,created_at)
        VALUES('move-out','ws','p','cold','out','transfer_out','BTC',1001,'2025-01-01T00:00:00Z','Move','fixed'),
              ('move-in','ws','p','hot','earn','transfer_in','BTC',1000,'2025-01-01T00:00:00Z','Move','fixed');
    """
    specs.append(('booked-transfer', 'reports.export-csv', transfer_sql, {}))
    for leg in ('out', 'earn'):
        specs.append(('conversion-foreign-' + leg, 'reports.export-csv',
                      f"UPDATE transactions SET profile_id='other' WHERE id='{leg}'; "
                      "UPDATE profiles SET last_processed_tx_count=3 WHERE id='p';", {}))
    specs.append(('booked-transfer-exact-first', 'reports.export-csv', """
        INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,transaction_id,entry_type,asset,quantity,occurred_at,description,created_at)
        VALUES('move-out','ws','p','cold','out','transfer_out','BTC',1002,'2025-01-01T00:00:00Z','Move','fixed'),
              ('move-out-2','ws','p','hot','no-tax','transfer_out','BTC',1000,'2025-01-01T00:00:00Z','Move','fixed'),
              ('move-in','ws','p','hot','earn','transfer_in','BTC',1001,'2025-01-01T00:00:00Z','Move','fixed'),
              ('move-in-2','ws','p','cold','in','transfer_in','BTC',1000,'2025-01-01T00:00:00Z','Move','fixed');
    """, {}))
    specs += [('algorithm-' + algorithm, 'reports.export-csv',
               f"UPDATE profiles SET gains_algorithm='{algorithm}' WHERE id='p';", {})
              for algorithm in ('FIFO', 'HIFO', 'MOVING_AVERAGE_AT')]
    specs += [("transactions-" + name, "reports.export-transactions-csv", sql, args)
              for name, command, sql, args in list(specs) if command == "reports.export-csv" and not name.startswith("history-limit-")]
    specs += [("transactions-wallets-" + str(i), "reports.export-transactions-csv", "", {"wallets": refs})
              for i, refs in enumerate([[], ["cold"], ["hot", "cold"], ["cold", "cold"], ["missing"]])]
    from kassiber.core.export_range import resolve_export_range
    specs += [("transactions-range-" + str(i), "reports.export-transactions-csv", sql,
               {"range_input": {"first_day": "2025-01-01", "last_day": "2025-12-31", "timezone_name": zone}})
              for i, (zone, sql) in enumerate([("UTC", ""), ("Europe/Vienna", ""),
                  ("UTC", "UPDATE transactions SET occurred_at='1970-01-01T00:00:00Z' WHERE id='out';"),
                  ("UTC", "UPDATE profiles SET last_processed_input_version=-1 WHERE id='p';")])]
    extra_wallets = "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('third','ws','p','Third','manual','{}','fixed'),('fourth','ws','p','Fourth','manual','{}','fixed');"
    specs += [('transactions-wallet-subset', 'reports.export-transactions-csv', extra_wallets +
               "UPDATE transactions SET wallet_id='third' WHERE id='no-tax';", {'wallets':['hot','cold']})]
    specs += [('transactions-three-wallet-label', 'reports.export-transactions-csv', extra_wallets,
               {'wallets':['hot','cold','third']})]
    specs += [('transactions-four-wallet-label', 'reports.export-transactions-csv', extra_wallets,
               {'wallets':['hot','cold','third','fourth']})]
    specs += [('transactions-range-boundaries', 'reports.export-transactions-csv',
               "UPDATE transactions SET occurred_at=CASE id WHEN 'in' THEN '2025-01-01T00:00:00Z' WHEN 'out' THEN '2026-01-01T00:00:00Z' ELSE occurred_at END;",
               {'range_input':{'first_day':'2025-01-01','last_day':'2025-12-31','timezone_name':'UTC'}})]
    result = []
    for name, command, sql, args in specs:
        with tempfile.TemporaryDirectory(prefix='kassiber-csv-oracle-') as root:
            conn = open_db(root)
            conn.executescript((OUT.parent / 'book.sql').read_text() + sql)
            hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
            try:
                if command == 'reports.export-transactions-csv':
                    export_range = None
                    if 'range_input' in args:
                        export_range = resolve_export_range(dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()), **args['range_input'])
                        args = {"export_range": export_range.payload()}
                    context = reports._transactions_export_context(conn, None, None, hooks,
                        wallet_ref=args.get('wallet'), wallet_refs=args.get('wallets'), export_range=export_range)
                    spec = context['spec']
                    files = [dict(name='report.csv', sheet='', rows=len(spec['rows']),
                        content=encode(reports._generic_report_csv_rows({'title':context['title']}, [spec])))]
                    metadata = dict(title=context['title'], wallet=reports.export_wallet_scope_label(context['wallets']),
                        rows=len(spec['rows']), warnings=context['warnings'], range=context['range'])
                elif command == 'reports.export-csv':
                    context = reports._build_full_report_context(conn, None, None, hooks,
                        wallet_ref=args.get('wallet'), history_limit=args.get('history_limit'))
                    sections = reports._generic_report_section_specs(context)
                    count = sum(len(section['rows']) for section in sections)
                    files = [dict(name='report.csv', sheet='', rows=count,
                                  content=encode(reports._generic_report_csv_rows(context, sections)))]
                    metadata = dict(title=context['title'], wallet=args.get('wallet') or '',
                                    sections=[section['sheet_name'] for section in sections], rows=count)
                else:
                    # Export first admits the ordinary context, then validates Austria.
                    context = reports._require_report_context(conn, None, None, hooks)
                    report = reports.report_austrian_e1kv(conn, None, None, hooks,
                        tax_year=args.get('year'), report_context=context)
                    raw = [('00_uebersicht.csv', 'Übersicht', reports._austrian_e1kv_overview_csv_rows(report))]
                    raw += [(spec['filename'], spec['sheet_name'], reports._austrian_e1kv_detail_csv_rows(spec))
                            for spec in reports._austrian_e1kv_section_table_specs(report)]
                    raw.append(('99_erlaeuterungen_zum_steuerreport.csv', 'Erläuterungen zum Steuerreport',
                                reports._austrian_e1kv_explanation_csv_rows(report)))
                    files = [dict(name=name, sheet=sheet, rows=max(len(rows)-1, 0), content=encode(rows))
                             for name, sheet, rows in raw]
                    metadata = dict(form=report['form'], tax_year=report['tax_year'],
                                    sheets=list(reports.AUSTRIAN_E1KV_XLSX_SHEETS), rows=len(report['rows']),
                                    summary_rows=len(report['summary_rows']))
                expected = dict(ok=dict(files=files, metadata=metadata))
            except AppError as exc:
                expected = dict(error=dict(code=exc.code, message=str(exc), hint=exc.hint,
                                           details=exc.details, retryable=exc.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as exc:
                expected = dict(error=dict(code='internal_error', message=str(exc), hint=None,
                                           details=None, retryable=False))
            conn.close()
            result.append(dict(name=name, command=command, sql=sql, args_json=json.dumps(args), expected=expected))
    return json.dumps(result, ensure_ascii=True, indent=2) + '\n'


if __name__ == '__main__':
    with patch.object(reports, 'datetime', FixedDateTime):
        content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'CSV artifact oracle drifted'
    else:
        OUT.write_text(content)
