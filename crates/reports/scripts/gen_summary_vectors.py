#!/usr/bin/env python3
"""Freeze Python's summary report model for the Rust report design.

``model`` is exactly the dict ``build_summary_pdf_report_data`` returns (the
data ``export_summary_pdf_report`` renders); refusals are kept as errors.
"""
from dataclasses import replace
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.db import open_db
from kassiber.errors import AppError

OUT = ROOT / 'crates/reports/tests/data/summary_model.json'
STAMP = '2026-01-02T03:04:05Z'


def monthly(count, start_year=2024):
    """Priced activity across ``count`` months, acquisitions and rates."""
    tx, je, rates = [], [], []
    for i in range(count):
        year, month = start_year + i // 12, i % 12 + 1
        day = f"{year:04d}-{month:02d}-15T12:00:00Z"
        rate = 30000 + i * 1750 - (i % 5) * 900
        direction = 'inbound' if i % 3 else 'outbound'
        wallet = 'cold' if i % 2 else 'hot'
        amount = 1000000000 + i * 37000000
        tx.append(f"('m{i}','ws','p','{wallet}','m{i}','BTC','{direction}',{amount},{1000 + i},{rate},'{rate}',NULL,NULL,0,'{day}','m{i:03d}',NULL,'deposit',NULL)")
        if direction == 'inbound':
            je.append(f"('ma{i}','ws','p','m{i}','{wallet}','acct','{day}','acquisition','BTC',{amount},{amount * rate / 1e11:.2f},{amount * rate / 1e11:.2f},NULL,NULL,NULL,NULL,NULL,'Buy','m{i:03d}')")
        else:
            gain = (i % 4 - 1.5) * 1234.5
            je.append(f"('md{i}','ws','p','m{i}','{wallet}','acct','{day}','disposal','BTC',{-amount},{amount * rate / 1e11:.2f},{amount * rate / 1e11 - gain:.2f},{amount * rate / 1e11:.2f},{gain},NULL,NULL,'short_term','Sell','m{i:03d}')")
        rates.append(f"('BTC-EUR','{year:04d}-{month:02d}-01T00:00:00Z',{rate},'{rate}','fixture','{day}')")
    return ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,"
            "fiat_rate,fiat_rate_exact,fiat_value,fiat_value_exact,excluded,occurred_at,created_at,note,kind,"
            f"taxability_override) VALUES {','.join(tx)};"
            "INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,"
            "entry_type,asset,quantity,fiat_value,cost_basis,proceeds,gain_loss,at_category,at_kennzahl,"
            f"capital_gains_type,description,created_at) VALUES {','.join(je)};"
            "INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES "
            f"{','.join(rates)};"
            f"UPDATE profiles SET last_processed_tx_count={4 + count} WHERE id='p';")


EXTRA_WALLETS = ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,account_id,created_at) VALUES "
                 + ",".join(f"('w{i}','ws','p','Wallet {i} with a long label','custom','{{}}','acct','fixed')" for i in range(8))
                 + ";INSERT INTO journal_wallet_holdings(id,workspace_id,profile_id,wallet_id,wallet_label,account_code,"
                 "asset,quantity,cost_basis,created_at) VALUES "
                 + ",".join(f"('h{i}','ws','p','w{i}','Wallet {i} with a long label','treasury','BTC',{(i + 1) * 3100000000},{(i + 1) * 900.5},'fixed')" for i in range(8))
                 + ";")


def specs():
    full = {'start': '2024-01-01T00:00:00Z', 'end': '2025-12-31T23:59:59Z'}
    return [
        ('default-period', '', {}),
        ('base-two-years', '', full),
        ('base-snapshot', '', {**full, 'include_snapshot': True}),
        ('one-wallet', '', {**full, 'wallets': ['cold']}),
        ('wallet-labels', '', {**full, 'wallets': ['Hot', ' cold ', 'hot']}),
        ('missing-wallet', '', {**full, 'wallets': ['absent']}),
        ('empty-wallets', '', {**full, 'wallets': ['  ']}),
        ('reversed', '', {'start': '2026-01-01T00:00:00Z', 'end': '2025-01-01T00:00:00Z'}),
        ('bad-start', '', {'start': 'yesterday'}),
        ('bad-end', '', {'end': '2025-13-01'}),
        ('offset-end', '', {'start': '2024-12-01T10:00:00+02:00', 'end': '2025-02-03T04:05:06.789-05:00'}),
        ('rich-ten-months', monthly(10), {'start': '2024-01-01T00:00:00Z', 'end': '2024-10-31T00:00:00Z'}),
        ('rich-fifteen-months', monthly(15), {'start': '2024-01-01T00:00:00Z', 'end': '2025-03-31T00:00:00Z'}),
        ('rich-thirty-months', monthly(30), {'start': '2024-01-01T00:00:00Z', 'end': '2026-06-15T12:00:00Z',
                                            'include_snapshot': True}),
        ('rich-partial-month', monthly(14), {'start': '2024-02-10T00:00:00Z', 'end': '2025-02-10T00:00:00Z'}),
        ('rich-cold', monthly(20), {'start': '2024-01-01T00:00:00Z', 'end': '2025-08-31T00:00:00Z', 'wallets': ['cold']}),
        ('many-wallets', EXTRA_WALLETS + monthly(6), {'start': '2024-01-01T00:00:00Z', 'end': '2024-06-30T00:00:00Z',
                                                      'include_snapshot': True}),
        ('no-rates-no-fiat', "UPDATE profiles SET fiat_currency='' WHERE id='p';", full),
        ('usd', "UPDATE profiles SET fiat_currency='usd' WHERE id='p';", full),
        ('negative-history', "UPDATE journal_wallet_holdings SET cost_basis=-500000.25 WHERE id='wz';"
                             "UPDATE journal_entries SET gain_loss=-1234567.891 WHERE id='z';", full),
        ('quarantine-internal', "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) "
                                "VALUES('in','ws','p','missing_price','fixed'),('earn','ws','p','unsupported_fee','fixed');"
                                "UPDATE transactions SET fiat_value=1234.5 WHERE id='out';", full),
        ('stale', "UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p';", full),
        ('dust', "UPDATE journal_wallet_holdings SET quantity=12345 WHERE id IN ('wz','wa');", full),
    ]


def build():
    cases = []
    for name, sql, args in specs():
        with tempfile.TemporaryDirectory(prefix='kassiber-summary-pdf-oracle-') as root:
            conn = open_db(root)
            conn.executescript((OUT.parent / 'book.sql').read_text() + sql)
            hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
            try:
                context = reports._require_report_context(conn, None, None, hooks)
                report = reports.build_summary_pdf_report_data(
                    conn, None, None, hooks, start=args.get('start'), end=args.get('end'),
                    wallet_refs=args.get('wallets'), include_snapshot=args.get('include_snapshot', False),
                    report_context=context)
                expected = dict(ok=json.loads(json.dumps(report)))
            except AppError as exc:
                expected = dict(error=dict(code=exc.code, message=str(exc), hint=exc.hint,
                                           details=exc.details, retryable=exc.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as exc:
                expected = dict(error=dict(code='internal_error', message=str(exc), hint=None,
                                           details=None, retryable=False))
            conn.close()
            cases.append(dict(name=name, sql=sql, args_json=json.dumps(args), expected=expected))
    return '[\n' + ',\n'.join(json.dumps(case, ensure_ascii=True, separators=(',', ':')) for case in cases) + '\n]\n'


if __name__ == '__main__':
    # The writer receives an explicit clock; the summary admits only datetimes,
    # so the module's datetime type must stay the real one.
    content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'summary model oracle drifted'
    else:
        OUT.write_text(content)
