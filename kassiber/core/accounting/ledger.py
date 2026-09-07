"""Country-neutral, exact-money general ledger with caller-owned transactions.

No wallet, RP2 or tax authority lives here. Positive debits and credits are
functional-currency minor units; quantities require a separate reviewed source.
"""
from __future__ import annotations

from contextlib import contextmanager
from datetime import date
import base64
import hashlib
import json
import re
from uuid import uuid4
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from ...errors import AppError
from ...secrets.sqlcipher import verify_unlock
from ...time_utils import now_iso

MAX_MINOR = 2**63 - 1
ACCOUNT_KINDS = frozenset(('asset', 'liability', 'equity', 'income', 'expense'))


def _error(message, code='accounting_validation'):
    raise AppError(message, code=code)


def strict_minor(value):
    if type(value) is not int or not 0 <= value <= MAX_MINOR:
        _error('Amounts must be nonnegative signed-64-bit integer minor units')
    return value


validate_minor = strict_minor


def _text(value, field, *, maximum=1000):
    if not isinstance(value, str) or not value.strip() or len(value) > maximum:
        _error(f'{field} must be nonempty text of at most {maximum} characters')
    return value.strip()


def _date(value):
    try:
        if not isinstance(value, str) or date.fromisoformat(value).isoformat() != value:
            raise ValueError()
    except ValueError:
        _error('Dates must be canonical YYYY-MM-DD strings')
    return value


def canonical_json(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False)


def digest(value):
    return hashlib.sha256(canonical_json(value).encode()).hexdigest()


@contextmanager
def atomic(conn):
    """Rollback the operation only, never commit the caller's transaction."""
    if not conn.in_transaction:
        conn.execute('BEGIN IMMEDIATE')
    name = 'gl_' + uuid4().hex
    conn.execute(f'SAVEPOINT {name}')
    try:
        yield
    except BaseException:
        conn.execute(f'ROLLBACK TO {name}')
        conn.execute(f'RELEASE {name}')
        raise
    else:
        conn.execute(f'RELEASE {name}')


def require_encrypted(conn):
    """Check keyed SQLCipher, not merely availability of the cipher extension."""
    status = conn.execute('PRAGMA cipher_status').fetchone()
    salt = conn.execute('PRAGMA cipher_salt').fetchone()
    # cipher_status was introduced in SQLCipher 4.12. Older versions expose
    # cipher_salt only when a keyed codec exists. Neither proves that the key
    # decrypts the database: require a real schema read as well. An explicit
    # failed status must never fall back to the older-version check.
    if (status is not None and str(status[0]) != '1') or not salt or not re.fullmatch('[0-9a-fA-F]{32}', str(salt[0])):
        _error('Accounting requires an unlocked encrypted SQLCipher database', 'accounting_requires_encryption')
    try:
        verify_unlock(conn)
    except AppError:
        _error('Accounting requires an unlocked encrypted SQLCipher database', 'accounting_requires_encryption')


def _row(conn, sql, params=()):
    cursor = conn.execute(sql, params)
    result = cursor.fetchone()
    return dict(zip((c[0] for c in cursor.description), result)) if result is not None else None


def _rows(conn, sql, params=()):
    cursor = conn.execute(sql, params)
    columns = [c[0] for c in cursor.description]
    return [dict(zip(columns, row)) for row in cursor.fetchall()]


def require_book(conn, profile_id):
    require_encrypted(conn)
    book = _row(conn, 'SELECT * FROM gl_books WHERE profile_id=?', (profile_id,))
    if not book:
        _error('Accounting is not enabled for this profile', 'accounting_not_enabled')
    return book


def _bump(conn, profile_id):
    conn.execute('UPDATE gl_books SET revision=revision+1 WHERE profile_id=?', (profile_id,))


def configure_book(conn, profile_id, *, currency, timezone, minor_unit_exponent=2,
                   entity_kind='organization', accounting_regime='accrual'):
    require_encrypted(conn)
    if not conn.execute('SELECT 1 FROM profiles WHERE id=?', (profile_id,)).fetchone():
        _error('Profile was not found', 'not_found')
    if not isinstance(currency, str) or not re.fullmatch('[A-Z]{3}', currency):
        _error('Functional currency must be a three-letter uppercase code')
    if type(minor_unit_exponent) is not int or not 0 <= minor_unit_exponent <= 8:
        _error('Currency exponent must be an integer from zero to eight')
    try:
        ZoneInfo(timezone)
    except (ZoneInfoNotFoundError, TypeError, ValueError):
        _error('A valid IANA timezone is required')
    if not isinstance(accounting_regime, str) or accounting_regime not in {'accrual', 'cash_basis'}:
        _error('Choose accrual or cash_basis accounting', 'accounting_unsupported_regime')
    config = dict(currency=currency, minor_unit_exponent=minor_unit_exponent, timezone=timezone,
                  entity_kind=_text(entity_kind, 'entity_kind', maximum=64),
                  accounting_regime=_text(accounting_regime, 'accounting_regime', maximum=64))
    with atomic(conn):
        existing = _row(conn, 'SELECT * FROM gl_books WHERE profile_id=?', (profile_id,))
        if existing:
            if any(existing[key] != value for key, value in config.items()):
                _error('Book configuration is immutable; use a separately scoped book', 'accounting_config_conflict')
        else:
            conn.execute('INSERT INTO gl_books(profile_id,currency,minor_unit_exponent,timezone,entity_kind,accounting_regime,created_at) VALUES(?,?,?,?,?,?,?)',
                         (profile_id, currency, minor_unit_exponent, timezone, config['entity_kind'], config['accounting_regime'], now_iso()))
    return book_status(conn, profile_id)


def book_status(conn, profile_id):
    book = _row(conn, 'SELECT * FROM gl_books WHERE profile_id=?', (profile_id,))
    if not book:
        return {'configured': False, 'profile_id': profile_id}
    require_encrypted(conn)
    return {'configured': True, 'profile_id': profile_id, 'book': book, 'revision': book['revision'],
            'accounts': _rows(conn, 'SELECT * FROM gl_accounts WHERE profile_id=? ORDER BY code', (profile_id,)),
            'periods': _rows(conn, 'SELECT * FROM gl_periods WHERE profile_id=? ORDER BY start_date', (profile_id,)),
            'closes': _rows(conn, "SELECT id,period_id,revision,created_at,snapshot_digest FROM gl_period_events WHERE profile_id=? AND action='close' ORDER BY created_at DESC,id", (profile_id,)),
            'drafts': journal(conn, profile_id, status='draft')}


snapshot = book_status


def create_account(conn, profile_id, *, code, name, kind):
    require_book(conn, profile_id)
    code, name = _text(code, 'code', maximum=64), _text(name, 'name', maximum=200)
    if kind not in ACCOUNT_KINDS:
        _error('Unknown account kind')
    with atomic(conn):
        existing = _row(conn, 'SELECT * FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, code))
        if existing:
            if existing['name'] != name or existing['kind'] != kind:
                _error('Account definitions are immutable', 'accounting_config_conflict')
            return existing
        conn.execute('INSERT INTO gl_accounts VALUES(?,?,?,?)', (profile_id, code, name, kind))
        _bump(conn, profile_id)
    return dict(profile_id=profile_id, code=code, name=name, kind=kind)


def create_period(conn, profile_id, *, period_id, start_date, end_date):
    require_book(conn, profile_id)
    period_id = _text(period_id, 'period_id', maximum=64)
    _date(start_date)
    _date(end_date)
    if start_date > end_date:
        _error('Fiscal period start must not follow end')
    with atomic(conn):
        existing = _row(conn, 'SELECT * FROM gl_periods WHERE profile_id=? AND id=?', (profile_id, period_id))
        if existing:
            if existing['start_date'] != start_date or existing['end_date'] != end_date:
                _error('Fiscal interval is immutable', 'accounting_config_conflict')
            return existing
        if conn.execute('SELECT 1 FROM gl_periods WHERE profile_id=? AND start_date<=? AND end_date>=?',
                        (profile_id, end_date, start_date)).fetchone():
            _error('Fiscal periods may not overlap')
        if conn.execute("SELECT 1 FROM gl_periods WHERE profile_id=? AND start_date>? AND state IN ('closed','review')",
                        (profile_id, end_date)).fetchone():
            _error('Reopen all later closed or review periods before adding an earlier fiscal period',
                   'accounting_later_period_closed')
        conn.execute('INSERT INTO gl_periods(profile_id,id,start_date,end_date) VALUES(?,?,?,?)',
                     (profile_id, period_id, start_date, end_date))
        _bump(conn, profile_id)
    return _period(conn, profile_id, period_id)


def _period(conn, profile_id, period_id, *, open_required=False):
    period_id = _text(period_id, 'period_id', maximum=200)
    period = _row(conn, 'SELECT * FROM gl_periods WHERE profile_id=? AND id=?', (profile_id, period_id))
    if not period:
        _error('Fiscal period not found in this book', 'not_found')
    if open_required and period['state'] != 'open':
        _error('Fiscal period is not open', 'accounting_period_closed')
    return period


def _normalize_payload(conn, profile_id, payload):
    if not isinstance(payload, dict):
        _error('Entry payload must be an object')
    unknown = set(payload) - {'idempotency_key','period_id','entry_date','description','lines','source_ref','entry_kind','reversal_of'}
    if unknown:
        _error('Unsupported posting fields: ' + ', '.join(sorted(unknown)))
    result = {key: _text(payload.get(key), key, maximum=200 if key != 'description' else 2000)
              for key in ('idempotency_key','period_id','entry_date','description')}
    _date(result['entry_date'])
    result['entry_kind'] = payload.get('entry_kind', 'normal')
    if result['entry_kind'] not in ('normal', 'opening', 'closing', 'reversal'):
        _error('Unsupported entry kind')
    result['source_ref'] = _text(payload['source_ref'], 'source_ref', maximum=300) if payload.get('source_ref') is not None else None
    result['reversal_of'] = _text(payload['reversal_of'], 'reversal_of', maximum=200) if payload.get('reversal_of') is not None else None
    lines = payload.get('lines')
    if not isinstance(lines, list) or not 2 <= len(lines) <= 1000:
        _error('An entry requires between two and one thousand lines')
    result['lines'] = []
    for line in lines:
        if not isinstance(line, dict) or set(line) - {'account_code','debit_minor','credit_minor'}:
            _error('Lines accept account_code, debit_minor and credit_minor only')
        account = _row(conn, 'SELECT * FROM gl_accounts WHERE profile_id=? AND code=?',
                       (profile_id, _text(line.get('account_code'), 'account_code', maximum=200)))
        if not account:
            _error('Posting account was not found in this book', 'not_found')
        debit, credit = strict_minor(line.get('debit_minor', 0)), strict_minor(line.get('credit_minor', 0))
        if (debit > 0) == (credit > 0):
            _error('Each line must have exactly one positive debit or credit')
        result['lines'].append(dict(account_code=account['code'], account_name=account['name'],
                                    account_kind=account['kind'], debit_minor=debit, credit_minor=credit))
    debit, credit = (sum(line[key] for line in result['lines']) for key in ('debit_minor', 'credit_minor'))
    strict_minor(debit)
    strict_minor(credit)
    if debit != credit:
        _error('Debits must equal credits exactly', 'accounting_unbalanced')
    return result


def _entry(conn, profile_id, entry_id):
    entry_id = _text(entry_id, 'entry_id', maximum=200)
    entry = _row(conn, 'SELECT * FROM gl_entries WHERE profile_id=? AND id=?', (profile_id, entry_id))
    if not entry:
        _error('Entry was not found in this book', 'not_found')
    entry['lines'] = _rows(conn, 'SELECT * FROM gl_lines WHERE profile_id=? AND entry_id=? ORDER BY position', (profile_id, entry_id))
    return entry


def _entry_payload(entry):
    result = {key: entry[key] for key in ('idempotency_key','period_id','entry_date','description','source_ref','entry_kind','reversal_of')}
    result['lines'] = [{key: line[key] for key in ('account_code','account_name','account_kind','debit_minor','credit_minor')}
                       for line in entry['lines']]
    return result


def _check_opening_cutoff(conn, profile_id, entry_date):
    opening = conn.execute("SELECT entry_date FROM gl_entries WHERE profile_id=? AND entry_kind='opening' AND status='posted'", (profile_id,)).fetchone()
    if opening and entry_date < opening[0]:
        _error('Entry predates the reviewed opening balances', 'accounting_before_opening')


def create_draft(conn, profile_id, payload):
    require_book(conn, profile_id)
    with atomic(conn):
        normalized = _normalize_payload(conn, profile_id, payload)
        payload_digest = digest(normalized)
        existing = _row(conn, 'SELECT id,payload_digest FROM gl_entries WHERE profile_id=? AND idempotency_key=?',
                        (profile_id, normalized['idempotency_key']))
        if existing:
            if existing['payload_digest'] != payload_digest:
                _error('Idempotency key already belongs to a different payload', 'accounting_idempotency_conflict')
            return _entry(conn, profile_id, existing['id'])
        period = _period(conn, profile_id, normalized['period_id'], open_required=True)
        _check_opening_cutoff(conn, profile_id, normalized['entry_date'])
        if not period['start_date'] <= normalized['entry_date'] <= period['end_date']:
            _error('Entry date is outside the fiscal period')
        if normalized['entry_kind'] == 'opening':
            if normalized['entry_date'] != period['start_date']:
                _error('Opening entry must be dated at the fiscal period start')
            if any(l['account_kind'] in ('income','expense') for l in normalized['lines']):
                _error('Opening entries may use balance sheet accounts only')
            if conn.execute("SELECT 1 FROM gl_entries WHERE profile_id=? AND status='posted'", (profile_id,)).fetchone():
                _error('Opening balances must precede other posted entries')
        elif normalized['entry_kind'] == 'reversal':
            original = _entry(conn, profile_id, normalized['reversal_of'])
            if conn.execute('SELECT 1 FROM gl_entries WHERE profile_id=? AND reversal_of=?',
                            (profile_id, original['id'])).fetchone():
                _error('This entry already has a reversal or a pending reversal draft',
                       'accounting_already_reversed')
            _require_reversible(conn, profile_id, original['id'])
            if normalized['entry_date'] < original['entry_date']:
                _error('A reversal cannot precede the original entry')
            expected = [dict(account_code=l['account_code'], account_name=l['account_name'], account_kind=l['account_kind'],
                             debit_minor=l['credit_minor'], credit_minor=l['debit_minor']) for l in original['lines']]
            if original['status'] != 'posted' or expected != normalized['lines']:
                _error('A reversal must exactly reverse a posted entry')
        elif normalized['reversal_of'] is not None:
            _error('Only reversal entries may reference reversal_of')
        entry_id = uuid4().hex
        conn.execute('INSERT INTO gl_entries(id,profile_id,period_id,entry_date,description,entry_kind,status,idempotency_key,payload_digest,source_ref,reversal_of,created_at) VALUES(?,?,?,?,?,?,\'draft\',?,?,?,?,?)',
                     (entry_id, profile_id, normalized['period_id'], normalized['entry_date'], normalized['description'],
                      normalized['entry_kind'], normalized['idempotency_key'], payload_digest, normalized['source_ref'], normalized['reversal_of'], now_iso()))
        for position, line in enumerate(normalized['lines']):
            conn.execute('INSERT INTO gl_lines VALUES(?,?,?,?,?,?,?,?,?)',
                         (uuid4().hex, entry_id, position, profile_id, line['account_code'], line['account_name'],
                          line['account_kind'], line['debit_minor'], line['credit_minor']))
        _bump(conn, profile_id)
    return _entry(conn, profile_id, entry_id)


def post_draft(conn, profile_id, *, draft_id, expected_digest):
    require_book(conn, profile_id)
    with atomic(conn):
        entry = _entry(conn, profile_id, draft_id)
        if expected_digest != entry['payload_digest'] or digest(_entry_payload(entry)) != expected_digest:
            _error('Posting approval does not match the current draft', 'accounting_stale_approval')
        if entry['status'] == 'posted':
            return entry
        _period(conn, profile_id, entry['period_id'], open_required=True)
        _check_opening_cutoff(conn, profile_id, entry['entry_date'])
        if entry['entry_kind'] == 'opening' and conn.execute("SELECT 1 FROM gl_entries WHERE profile_id=? AND status='posted'", (profile_id,)).fetchone():
            _error('Opening balances must be posted before any other entry')
        if entry['entry_kind'] == 'closing':
            _validate_closing(conn, profile_id, entry)
        if entry['entry_kind'] == 'reversal':
            _require_reversible(conn, profile_id, entry['reversal_of'])
        projection_enabled = conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_projection_proposals'").fetchone()
        if projection_enabled:
            from . import projection
            projection.validate_draft(conn, profile_id, entry)
        valuation_enabled = conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_book_valuations'").fetchone()
        if valuation_enabled:
            from . import valuation
            valuation.validate_draft(conn, profile_id, entry)
        conn.execute("UPDATE gl_entries SET status='posted',posted_at=? WHERE id=? AND profile_id=?", (now_iso(), draft_id, profile_id))
        if projection_enabled:
            projection.after_post(conn, profile_id, draft_id)
            if entry['entry_kind'] == 'reversal':
                projection.after_reverse(conn, profile_id, entry['reversal_of'], draft_id)
        if valuation_enabled:
            valuation.after_post(conn, profile_id, draft_id)
            if entry['entry_kind'] == 'reversal':
                valuation.after_reverse(conn, profile_id, entry['reversal_of'], draft_id)
        _bump(conn, profile_id)
    return _entry(conn, profile_id, draft_id)


def discard_draft(conn, profile_id, *, draft_id, expected_digest):
    require_book(conn, profile_id)
    with atomic(conn):
        entry = _entry(conn, profile_id, draft_id)
        if entry['status'] != 'draft' or expected_digest != entry['payload_digest']:
            _error('Only the reviewed unposted draft may be discarded', 'accounting_stale_approval')
        if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_projection_proposals'").fetchone():
            from . import projection
            projection.before_discard(conn, profile_id, entry)
        if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_book_valuations'").fetchone():
            from . import valuation
            valuation.before_discard(conn, profile_id, entry)
        conn.execute('DELETE FROM gl_lines WHERE profile_id=? AND entry_id=?', (profile_id, draft_id))
        conn.execute('DELETE FROM gl_entries WHERE profile_id=? AND id=?', (profile_id, draft_id))
        _bump(conn, profile_id)
    return {'discarded': True, 'id': draft_id}


def _validate_closing(conn, profile_id, entry):
    period = _period(conn, profile_id, entry['period_id'])
    if entry['entry_date'] != period['end_date']:
        _error('Result appropriation must be dated at the fiscal period end')
    balances = {r['account_code']: r['balance_minor'] for r in trial_balance(conn, profile_id,
                period_id=entry['period_id'])['rows'] if r['kind'] in ('income','expense') and r['balance_minor']}
    movements = {}
    equity = 0
    for line in entry['lines']:
        value = line['debit_minor'] - line['credit_minor']
        if line['account_kind'] == 'equity':
            equity += value
        elif line['account_kind'] in ('income','expense'):
            movements[line['account_code']] = movements.get(line['account_code'], 0) + value
        else:
            _error('Result appropriation may only clear P&L accounts to equity')
    if movements != {code: -balance for code, balance in balances.items()} or equity != sum(balances.values()):
        _error('Result appropriation must clear every current period P&L balance exactly')


def _require_reversible(conn, profile_id, entry_id):
    if _entry(conn, profile_id, entry_id)['entry_kind'] == 'reversal':
        _error('A reversal cannot itself be reversed; create a reviewed correcting entry instead',
               'accounting_reversal_not_allowed')
    if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_projection_proposals'").fetchone():
        from .projection import require_reversible
        require_reversible(conn, profile_id, entry_id)
    if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_book_valuations'").fetchone():
        from . import valuation
        valuation.require_reversible(conn, profile_id, entry_id)
    for table, column, direct, void_table, void_key in (
        ('gl_bank_allocations','line_id',False,'gl_bank_allocation_voids','allocation_id'),
        ('gl_open_items','origin_line_id',False,'gl_open_item_voids','item_id'),
        ('gl_open_item_allocations','line_id',False,'gl_open_item_allocation_voids','allocation_id'),
        ('gl_schedule_revisions','entry_id',True,None,None),
    ):
        if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)).fetchone():
            condition = f'{column}=?' if direct else f'{column} IN (SELECT id FROM gl_lines WHERE entry_id=?)'
            if void_table and conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (void_table,)).fetchone():
                condition += f' AND NOT EXISTS(SELECT 1 FROM {void_table} v WHERE v.profile_id={table}.profile_id AND v.{void_key}={table}.id)'
            if table == 'gl_schedule_revisions':
                condition += ' AND revision=(SELECT MAX(r.revision) FROM gl_schedule_revisions r WHERE r.schedule_id=gl_schedule_revisions.schedule_id AND r.profile_id=gl_schedule_revisions.profile_id AND r.effective_date=gl_schedule_revisions.effective_date)'
            if conn.execute(f'SELECT 1 FROM {table} WHERE profile_id=? AND {condition}', (profile_id, entry_id)).fetchone():
                _error('Referenced allocations or schedules require a reviewed correction workflow before reversal', 'accounting_entry_retained')


def reverse_entry(conn, profile_id, *, entry_id, entry_date, period_id, idempotency_key, reason):
    require_book(conn, profile_id)
    with atomic(conn):
        entry = _entry(conn, profile_id, entry_id)
        payload = dict(idempotency_key=idempotency_key, period_id=period_id, entry_date=entry_date,
                       description=_text(reason, 'reason'), entry_kind='reversal', reversal_of=entry_id,
                       lines=[dict(account_code=l['account_code'], debit_minor=l['credit_minor'], credit_minor=l['debit_minor'])
                              for l in entry['lines']])
        draft = create_draft(conn, profile_id, payload)
        return post_draft(conn, profile_id, draft_id=draft['id'], expected_digest=draft['payload_digest'])


def journal(conn, profile_id, *, period_id=None, status='posted', limit=500):
    require_book(conn, profile_id)
    if status not in ('posted','draft') or type(limit) is not int or not 1 <= limit <= 5000:
        _error('Invalid journal bounds or status')
    clauses, params = ['profile_id=?', 'status=?'], [profile_id, status]
    if period_id is not None:
        _period(conn, profile_id, period_id)
        clauses.append('period_id=?')
        params.append(period_id)
    rows = _rows(conn, 'SELECT id FROM gl_entries WHERE ' + ' AND '.join(clauses) + ' ORDER BY entry_date,created_at,id LIMIT ?', (*params, limit))
    return [_entry(conn, profile_id, row['id']) for row in rows]


def journal_page(conn, profile_id, *, period_id=None, status='posted', limit=100, cursor=None):
    """Bounded keyset pages pinned to one book revision and exact filter scope."""
    book = require_book(conn, profile_id)
    if status not in ('posted', 'draft') or type(limit) is not int or not 1 <= limit <= 500:
        _error('Invalid journal bounds or status')
    binding = dict(profile_id=profile_id, period_id=period_id, status=status, revision=book['revision'])
    clauses, params = ['profile_id=?', 'status=?'], [profile_id, status]
    if period_id is not None:
        _period(conn, profile_id, period_id)
        clauses.append('period_id=?')
        params.append(period_id)
    if cursor is not None:
        try:
            if not isinstance(cursor, str) or len(cursor) > 4096:
                raise ValueError()
            decoded = json.loads(base64.b64decode(cursor, altchars=b'-_', validate=True))
            if not isinstance(decoded, dict) or set(decoded) != set(binding) | {'after'}:
                raise ValueError()
            if any(decoded[key] != value for key, value in binding.items()):
                raise ValueError()
            after = decoded['after']
            if not isinstance(after, list) or len(after) != 3 or any(not isinstance(v, str) or len(v) > 128 for v in after):
                raise ValueError()
            anchor = conn.execute('SELECT 1 FROM gl_entries WHERE ' + ' AND '.join(clauses) + ' AND entry_date=? AND created_at=? AND id=?', (*params, *after)).fetchone()
            if not anchor:
                raise ValueError()
        except (ValueError, TypeError, RecursionError):
            _error('Journal cursor is invalid or the book changed; reload from the first page', 'accounting_stale_cursor')
        clauses.append('(entry_date,created_at,id)>(?,?,?)')
        params.extend(after)
    rows = _rows(conn, 'SELECT id,entry_date,created_at FROM gl_entries WHERE ' + ' AND '.join(clauses) + ' ORDER BY entry_date,created_at,id LIMIT ?', (*params, limit + 1))
    visible = rows[:limit]
    entries = [_entry(conn, profile_id, row['id']) for row in visible]
    if require_book(conn, profile_id)['revision'] != book['revision']:
        _error('Book changed while reading journal; reload', 'accounting_stale_cursor')
    next_cursor = None
    if len(rows) > limit:
        last = visible[-1]
        token = dict(binding, after=[last['entry_date'], last['created_at'], last['id']])
        next_cursor = base64.urlsafe_b64encode(canonical_json(token).encode()).decode()
    return dict(entries=entries, next_cursor=next_cursor, revision=book['revision'])


def trial_balance(conn, profile_id, *, period_id=None, as_of=None, exclude_closing=False):
    book = require_book(conn, profile_id)
    clauses, params = ["e.profile_id=?", "e.status='posted'"], [profile_id]
    if period_id is not None:
        _period(conn, profile_id, period_id)
        clauses.append('e.period_id=?')
        params.append(period_id)
    if as_of is not None:
        clauses.append('e.entry_date<=?')
        params.append(_date(as_of))
    if exclude_closing:
        clauses.append("e.entry_kind!='closing' AND NOT EXISTS(SELECT 1 FROM gl_entries original WHERE original.id=e.reversal_of AND original.entry_kind='closing')")
    totals = {}
    # Python accumulation is exact even when lifetime turnover exceeds SQLite SUM's integer range.
    for row in _rows(conn, 'SELECT l.* FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE ' + ' AND '.join(clauses), params):
        item = totals.setdefault(row['account_code'], dict(account_code=row['account_code'], name=row['account_name'], kind=row['account_kind'], debit_minor=0, credit_minor=0))
        item['debit_minor'] += row['debit_minor']
        item['credit_minor'] += row['credit_minor']
    rows = []
    for code in sorted(totals):
        item = totals[code]
        item['balance_minor'] = item['debit_minor'] - item['credit_minor']
        rows.append(item)
    debit, credit = sum(r['debit_minor'] for r in rows), sum(r['credit_minor'] for r in rows)
    return dict(profile_id=profile_id, period_id=period_id, as_of=as_of, currency=book['currency'],
                minor_unit_exponent=book['minor_unit_exponent'], rows=rows, debit_minor=debit,
                credit_minor=credit, balanced=debit == credit, revision=book['revision'])


def financial_statements(conn, profile_id, *, period_id):
    require_book(conn, profile_id)
    period = _period(conn, profile_id, period_id)
    movement = trial_balance(conn, profile_id, period_id=period_id, exclude_closing=True)
    cumulative = trial_balance(conn, profile_id, as_of=period['end_date'])
    pnl = [r for r in movement['rows'] if r['kind'] in ('income','expense')]
    balance_sheet = [r for r in cumulative['rows'] if r['kind'] not in ('income','expense')]
    unappropriated = -sum(r['balance_minor'] for r in cumulative['rows'] if r['kind'] in ('income','expense'))
    return dict(profile_id=profile_id, period_id=period_id, currency=movement['currency'],
                minor_unit_exponent=movement['minor_unit_exponent'], profit_and_loss=pnl,
                profit_minor=-sum(r['balance_minor'] for r in pnl), balance_sheet=balance_sheet,
                unappropriated_result_minor=unappropriated,
                balanced=sum(r['balance_minor'] for r in balance_sheet) == unappropriated,
                revision=movement['revision'])


def close_readiness(conn, profile_id, *, period_id):
    """Read the same local accounting controls enforced inside the close transaction."""
    book = require_book(conn, profile_id)
    period = _period(conn, profile_id, period_id)
    blockers = []
    if period['state'] != 'open':
        blockers.append({'kind': 'period_not_open'})
    drafts = conn.execute("SELECT COUNT(*) FROM gl_entries WHERE profile_id=? AND period_id=? AND status='draft'", (profile_id, period_id)).fetchone()[0]
    if drafts:
        blockers.append({'kind': 'unposted_drafts', 'count': drafts})
    earlier = conn.execute("SELECT COUNT(*) FROM gl_periods WHERE profile_id=? AND end_date<? AND state!='closed'", (profile_id, period['start_date'])).fetchone()[0]
    if earlier:
        blockers.append({'kind': 'earlier_periods_open', 'count': earlier})
    controls = {'coverage_verified': False, 'blockers': []}
    projection_controls = cash_controls = None
    if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_bank_statements'").fetchone():
        from .schedules import validate_close
        controls = validate_close(conn, profile_id, period['start_date'], period['end_date'])
        blockers.extend({'kind': 'supporting_record', **row} for row in controls['blockers'])
    if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_projection_proposals'").fetchone():
        from .projection import validate_close as projection_check
        projection_controls = projection_check(conn, profile_id, period['start_date'], period['end_date'])
        blockers.extend({'kind': 'source_projection', **row} for row in projection_controls['blockers'])
    if conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_cash_accounts'").fetchone():
        from .cashbook import validate_close as cash_check
        cash_controls = cash_check(conn, profile_id, period['start_date'], period['end_date'])
        blockers.extend({'kind': 'cash_book', **row} for row in cash_controls['blockers'])
    return dict(period_id=period_id, revision=book['revision'], ready=not blockers,
                blockers=blockers, controls=controls, projection_controls=projection_controls,
                cash_controls=cash_controls, external_completeness_verified=False,
                tax_filing_ready=False)


def close_period(conn, profile_id, *, period_id, expected_revision):
    require_book(conn, profile_id)
    with atomic(conn):
        book = require_book(conn, profile_id)
        if type(expected_revision) is not int or expected_revision != book['revision']:
            _error('Book changed after close review', 'accounting_stale_approval')
        period = _period(conn, profile_id, period_id, open_required=True)
        readiness = close_readiness(conn, profile_id, period_id=period_id)
        if readiness['blockers']:
            message = {
                'unposted_drafts': 'Resolve all fiscal-period drafts before closing',
                'earlier_periods_open': 'Earlier fiscal periods must be closed first',
            }.get(readiness['blockers'][0]['kind'], 'Accounting controls prevent close')
            raise AppError(message, code='accounting_close_blocked', details={'blockers': readiness['blockers']})
        controls = readiness['controls']
        projection_controls = readiness['projection_controls']
        cash_controls = readiness['cash_controls']
        state = dict(book=book, period=period, trial_balance=trial_balance(conn, profile_id, period_id=period_id),
                     statements=financial_statements(conn, profile_id, period_id=period_id),
                     entries=_rows(conn, "SELECT id,payload_digest FROM gl_entries WHERE profile_id=? AND period_id=? AND status='posted' ORDER BY id", (profile_id, period_id)))
        state['journal'] = [_entry(conn, profile_id, row['id']) for row in _rows(conn,
            "SELECT id FROM gl_entries WHERE profile_id=? AND status='posted' AND entry_date<=? ORDER BY entry_date,created_at,id",
            (profile_id, period['end_date']))]
        state['accounts'] = _rows(conn, 'SELECT * FROM gl_accounts WHERE profile_id=? ORDER BY code', (profile_id,))
        if projection_controls is not None:
            state['projection_controls'] = projection_controls
        if cash_controls is not None:
            state['cash_controls'] = cash_controls
        state['controls'] = controls
        state['state'] = 'accounting_closed'
        state['tax_workpaper_ready'] = False
        from .package import MAX_SNAPSHOT_BYTES
        if len(canonical_json(state).encode('utf-8')) > MAX_SNAPSHOT_BYTES:
            _error('Close snapshot exceeds the portable package limit; period remains open', 'accounting_close_too_large')
        revision = period['revision'] + 1
        result = _period_event(conn, profile_id, period_id, revision, 'close', 'Reviewed accounting close', state)
        conn.execute("UPDATE gl_periods SET state='closed',revision=? WHERE profile_id=? AND id=?", (revision, profile_id, period_id))
        _bump(conn, profile_id)
    return result


def _period_event(conn, profile_id, period_id, revision, action, reason, state):
    event_id, checksum = uuid4().hex, digest(state)
    conn.execute('INSERT INTO gl_period_events VALUES(?,?,?,?,?,?,?,?,?)',
                 (event_id, profile_id, period_id, revision, action, reason, canonical_json(state), checksum, now_iso()))
    return dict(id=event_id, period_id=period_id, revision=revision, action=action, snapshot_digest=checksum, snapshot=state)


def reopen_period(conn, profile_id, *, period_id, reason, expected_revision):
    require_book(conn, profile_id)
    reason = _text(reason, 'reason')
    with atomic(conn):
        book = require_book(conn, profile_id)
        if type(expected_revision) is not int or expected_revision != book['revision']:
            _error('Book changed after reopen review', 'accounting_stale_approval')
        period = _period(conn, profile_id, period_id)
        if period['state'] not in ('closed','review'):
            _error('Only closed or dependent-review periods can be reopened')
        revision = period['revision'] + 1
        result = _period_event(conn, profile_id, period_id, revision, 'reopen', reason, dict(book=book, period=period))
        conn.execute("UPDATE gl_periods SET state='open',revision=? WHERE profile_id=? AND id=?", (revision, profile_id, period_id))
        conn.execute("UPDATE gl_periods SET state='review' WHERE profile_id=? AND start_date>?", (profile_id, period['end_date']))
        _bump(conn, profile_id)
    return result
