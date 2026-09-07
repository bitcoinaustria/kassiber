"""Exact, revision-bound account ledger; no tax or wallet interpretation."""
from __future__ import annotations

import base64
import json

from ...errors import AppError
from .ledger import atomic, canonical_json, require_book, _period, _row


def account_ledger(conn, profile_id, *, account_code, period_id, limit=100, cursor=None):
    """Return a full-period control plus a bounded page with running balances.

    Monetary accumulation uses Python integers even when lifetime turnover exceeds
    SQLite's signed-64-bit SUM limit. Drafts never affect an account ledger.
    Cursors bind the book, fiscal period, account and current revision. A read
    savepoint keeps totals and page rows in one database snapshot.
    """
    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError('Account ledger limit must be between 1 and 500', code='accounting_validation')
    with atomic(conn):
        book = require_book(conn, profile_id)
        account = _row(conn, 'SELECT * FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, account_code))
        if not account:
            raise AppError('Account was not found in this book', code='not_found')
        period = _period(conn, profile_id, period_id)
        binding = dict(version=1, profile_id=profile_id, account_code=account_code,
                       period_id=period_id, revision=book['revision'])
        after = None
        if cursor is not None:
            try:
                if not isinstance(cursor, str) or len(cursor) > 4096:
                    raise ValueError()
                token = json.loads(base64.b64decode(cursor, altchars=b'-_', validate=True))
                if not isinstance(token, dict) or set(token) != {*binding, 'last'}:
                    raise ValueError()
                after = token.pop('last')
                if not isinstance(after, list) or len(after) != 3 or not all(isinstance(x, str) for x in after[:2]) or type(after[2]) is not int:
                    raise ValueError()
                if token != binding:
                    raise AppError('Account ledger changed; refresh from the first page', code='accounting_stale_cursor')
            except (ValueError, TypeError, UnicodeError) as exc:
                raise AppError('Invalid account ledger continuation', code='accounting_invalid_cursor') from exc
        opening = debit = credit = count = 0
        running = 0
        records = []
        seen_after = after is None
        for row in conn.execute('''SELECT e.id AS entry_id,e.entry_date,e.description,e.entry_kind,e.reversal_of,
                l.id AS line_id,l.position,l.debit_minor,l.credit_minor
            FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
            WHERE l.profile_id=? AND l.account_code=? AND e.status='posted' AND e.entry_date<=?
            ORDER BY e.entry_date,e.id,l.position''', (profile_id, account_code, period['end_date'])):
            movement = row['debit_minor'] - row['credit_minor']
            running += movement
            if row['entry_date'] < period['start_date']:
                opening += movement
                continue
            count += 1
            debit += row['debit_minor']
            credit += row['credit_minor']
            key = [row['entry_date'], row['entry_id'], row['position']]
            if after is not None and key == after:
                seen_after = True
            if after is not None and key <= after:
                continue
            if len(records) <= limit:
                records.append({**dict(row), 'running_balance_minor': running})
        if not seen_after:
            raise AppError('Account ledger continuation does not identify a line', code='accounting_invalid_cursor')
        next_cursor = None
        if len(records) > limit:
            records = records[:limit]
            tail = records[-1]
            token = {**binding, 'last': [tail['entry_date'], tail['entry_id'], tail['position']]}
            next_cursor = base64.urlsafe_b64encode(canonical_json(token).encode()).decode()
        return dict(account=account, period=period, currency=book['currency'],
                    minor_unit_exponent=book['minor_unit_exponent'], revision=book['revision'],
                    opening_minor=opening, debit_minor=debit, credit_minor=credit,
                    closing_minor=opening + debit - credit, rows=records,
                    total_count=count, next_cursor=next_cursor)
