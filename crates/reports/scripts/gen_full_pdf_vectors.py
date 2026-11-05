#!/usr/bin/env python3
"""Freeze Python's full report model for the shared report design.

``model`` holds the values ``export_pdf_report`` prints, read from the same
context (``_build_full_report_context``): exact, unformatted, in Python's
order. ``title`` is the envelope's title; refusals are kept as errors.
"""
from dataclasses import replace
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.db import open_db
from kassiber.errors import AppError

OUT = ROOT / 'crates/reports/tests/data/full_pdf.json'
STAMP = '2026-01-02T03:04:05Z'


class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc).astimezone(tz)


def many_transactions(count):
    values = ','.join(
        f"('bulk{i}','ws','p','{'hot' if i % 3 else 'cold'}','bulk{i}','BTC',"
        f"'{'inbound' if i % 2 else 'outbound'}',{1000 + i * 7919},{i},NULL,NULL,NULL,NULL,0,"
        f"'2025-03-{1 + i % 28:02d}T{i % 24:02d}:00:00Z','bulk{i:04d}',NULL,'deposit',NULL)"
        for i in range(count))
    return ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,"
            "fiat_rate,fiat_rate_exact,fiat_value,fiat_value_exact,excluded,occurred_at,created_at,note,kind,"
            f"taxability_override) VALUES {values};")


def specs():
    inherited = json.loads((OUT.parent / 'artifact_csv.json').read_text())
    result = [(case['name'], case['sql'], json.loads(case['args_json']))
              for case in inherited if case['command'] == 'reports.export-csv']
    long_word = 'x' * 300
    result += [
        ('long-lines', "UPDATE transactions SET description='" + ' '.join(['word'] * 60) + ' ' + long_word +
         "' WHERE id='out';", {}),
        ('whitespace', "UPDATE transactions SET description='a'||char(9)||'tab  two'||char(10)||'line'||char(13)||"
         "'cr'||char(11)||char(12)||' end ' WHERE id='out';", {}),
        ('unicode', "UPDATE transactions SET description='日本語 € ₿ → ← ↔ — – ‑ ⚠ … ‘’ “” ‚ „ ≈ • · ä 😀' "
         "WHERE id='out'; UPDATE wallets SET label='Cold (\\\\ € ))' WHERE id='cold';", {}),
        ('pdf-escapes', "UPDATE profiles SET label='Book (a) \\\\ b)' WHERE id='p';", {}),
        ('scoped-unicode', "UPDATE wallets SET label='Hot ₿ (x)' WHERE id='hot';", {'wallet': 'hot', 'history_limit': 2}),
        ('many-pages', many_transactions(130) + "UPDATE profiles SET last_processed_tx_count=134 WHERE id='p';", {}),
        ('wide-numbers', "UPDATE journal_wallet_holdings SET quantity=9123456789012345678,cost_basis=-98765432109.876 "
         "WHERE id='wz';", {}),
        ('backend-values', "UPDATE wallets SET config_json='{\"backend\":null,\"gap_limit\":true}' WHERE id='cold'; "
         "UPDATE wallets SET config_json='{\"backend\":{\"a\":[1,2.5]},\"gap_limit\":\"7\"}' WHERE id='hot';", {}),
        ('no-processing', "UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0 WHERE id='p';", {}),
        # Three capital buckets whose first-seen order differs from sorted order.
        ('capital-buckets', "UPDATE transactions SET taxability_override=NULL WHERE id='no-tax';", {}),
        ('label-width', "UPDATE wallets SET label='" + 'W' * 40 + "' WHERE id='cold';"
         "UPDATE journal_quarantines SET reason='" + 'r' * 40 + "';", {}),
    ]
    return result


def text(value):
    """The report's text for a stored value, as its tables print it."""
    return str(value)


def counted(value):
    return int(value or 0)


def flows(rows, prefix):
    return [dict(**{key: text(row[key]) for key in prefix}, tx_count=counted(row['tx_count']),
                 inbound_count=counted(row['inbound_count']), outbound_count=counted(row['outbound_count']),
                 inbound_msat=counted(row['inbound_amount']), outbound_msat=counted(row['outbound_amount']),
                 fee_msat=counted(row['fee_amount'])) for row in rows]


def full_model(context):
    """The full report's values for the shared design."""
    profile, summary, query = context['profile'], context['summary'], context['query_rows']
    pick = lambda row, keys: {key: row[key] for key in keys}
    return dict(
        title=context['pdf_title'],
        generated_at=context['generated_at'],
        workspace=context['workspace']['label'],
        profile=pick(profile, ('label', 'fiat_currency', 'tax_country', 'tax_long_term_days', 'gains_algorithm',
                               'last_processed_at')) | dict(
            last_processed_tx_count=counted(profile['last_processed_tx_count']),
            journal_input_version=counted(profile['journal_input_version'])),
        wallet=context['wallet']['label'] if context['wallet'] else None,
        metrics=dict(wallets_in_scope=len(context['scope_wallets']), assets_in_scope=counted(summary['asset_count']),
                     active_transactions=counted(summary['active_transactions']),
                     excluded_transactions=counted(summary['excluded_transactions']),
                     inbound_transactions=counted(summary['inbound_transactions']),
                     outbound_transactions=counted(summary['outbound_transactions']),
                     journal_entries=counted(query['journal_entries']), quarantines=counted(query['quarantines']),
                     priced_transactions=counted(summary['priced_transactions']),
                     transactions_with_notes=counted(summary['noted_transactions']),
                     transactions_with_tags=counted(query['tagged_transactions']),
                     first_transaction_at=summary['first_transaction_at'] or '',
                     last_transaction_at=summary['last_transaction_at'] or ''),
        rollups=context['rollups'],
        wallets=[{key: text(row[key]) for key in ('label', 'kind', 'chain', 'network', 'backend', 'gap_limit')}
                 for row in context['scope_wallets']],
        asset_flow=flows(query['flow_by_asset'], ('asset',)),
        wallet_flow=flows(query['flow_by_wallet'], ('wallet', 'asset')),
        balances=[dict(account=text(row['account']), asset=text(row['asset']))
                  | pick(row, ('quantity', 'cost_basis', 'market_value', 'unrealized_pnl'))
                  for row in context['balance_rows']],
        portfolio=[dict(wallet=text(row['wallet']), account=text(row['account']), asset=text(row['asset']))
                   | pick(row, ('quantity', 'avg_cost', 'cost_basis', 'market_value', 'unrealized_pnl'))
                   for row in context['portfolio_rows']],
        capital=[dict(occurred_at=text(row['occurred_at']), wallet=text(row['wallet']), asset=text(row['asset']),
                      entry_type=text(row['entry_type']))
                 | pick(row, ('quantity', 'proceeds', 'cost_basis', 'gain_loss'))
                 for row in context['capital_rows']],
        history=[dict(period_start=text(row['period_start']), asset=text(row['asset']))
                 | pick(row, ('quantity', 'cumulative_cost_basis', 'market_value'))
                 for row in context['history_rows']],
        quarantines=[dict(reason=text(row['reason']), count=counted(row['count']))
                     for row in query['quarantine_rows']],
        transactions=[dict(occurred_at=text(row['occurred_at']), wallet=text(row['wallet']),
                           direction=text(row['direction']), asset=text(row['asset']),
                           amount_msat=counted(row['amount']), fee_msat=counted(row['fee']),
                           description=text(row['description']))
                      for row in query['transactions']],
    )


def build():
    cases = []
    for name, sql, args in specs():
        with tempfile.TemporaryDirectory(prefix='kassiber-pdf-oracle-') as root:
            conn = open_db(root)
            conn.executescript((OUT.parent / 'book.sql').read_text() + sql)
            hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
            try:
                history_limit = args.get('history_limit')
                if history_limit is not None and int(history_limit) < 0:
                    raise AppError('--history-limit must be zero or positive', code='validation')
                context = reports._require_report_context(conn, None, None, hooks)
                model = json.loads(json.dumps(full_model(reports._build_full_report_context(
                    conn, None, None, hooks, wallet_ref=args.get('wallet'), history_limit=history_limit,
                    report_context=context))))
                title, _ = reports.build_pdf_report_lines(
                    conn, None, None, hooks, wallet_ref=args.get('wallet'),
                    history_limit=history_limit, report_context=context)
                expected = dict(ok=dict(title=title, model=model))
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
    with patch.object(reports, 'datetime', FixedDateTime):
        content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'full PDF oracle drifted'
    else:
        OUT.write_text(content)
