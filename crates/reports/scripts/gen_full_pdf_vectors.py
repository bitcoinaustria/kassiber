#!/usr/bin/env python3
"""Freeze Python's full text-PDF report model and document bytes.

Each case captures the exact ``(title, lines)`` that ``export_pdf_report``
hands to ``write_text_pdf`` and the bytes that writer produces, so the Rust
port is checked on both the report model and the hand-written PDF 1.4 file.
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
from kassiber.pdf_report import write_text_pdf

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
                title, lines = reports.build_pdf_report_lines(
                    conn, None, None, hooks, wallet_ref=args.get('wallet'),
                    history_limit=history_limit, report_context=context)
                written = write_text_pdf(Path(root) / 'report.pdf', title, lines)
                pdf = (Path(root) / 'report.pdf').read_bytes()
                expected = dict(ok=dict(title=title, lines=lines, pages=written['pages'],
                                        pdf=pdf.decode('latin-1')))
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
