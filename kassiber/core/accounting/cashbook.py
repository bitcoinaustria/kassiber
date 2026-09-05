"""Reviewed liquidity classifications and cash counts, not a tax calculation.

Cash-basis flows have exact allocation provenance on opposite posted GL lines.
They are separate from accrual profit. Canonical reversals invert the original
classification on their actual date; they never create another payment fact.
"""
from __future__ import annotations

from uuid import uuid4

from kassiber.errors import AppError
from . import ledger
from .bank import iso_date, require_open_interval
from .evidence import bounded_text, require_evidence

ROLES = frozenset({'cash', 'bank', 'loan'})
CLASSIFICATIONS = frozenset({'income', 'expenditure', 'non_result'})


def _error(message, code='accounting_cash_invalid'):
    raise AppError(message, code=code)


def _book(conn, profile_id):
    bounded_text(profile_id, 'profile_id', 200)
    return ledger.require_book(conn, profile_id)


def ensure_schema(conn):
    conn.execute('''CREATE TABLE IF NOT EXISTS gl_cash_accounts (
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,account_code TEXT NOT NULL,
        role TEXT NOT NULL CHECK(role IN ('cash','bank','loan')),effective_from TEXT NOT NULL,
        reason TEXT NOT NULL,idempotency_key TEXT NOT NULL,payload_digest TEXT NOT NULL,
        UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),
        FOREIGN KEY(profile_id,account_code) REFERENCES gl_accounts(profile_id,code))''')
    conn.execute('''CREATE TABLE IF NOT EXISTS gl_cash_counts (
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,selection_id TEXT NOT NULL,
        count_date TEXT NOT NULL,counted_minor INTEGER NOT NULL
        CHECK(typeof(counted_minor)='integer' AND counted_minor>=0),
        evidence_id TEXT NOT NULL,locator TEXT NOT NULL,reason TEXT NOT NULL,
        idempotency_key TEXT NOT NULL,payload_digest TEXT NOT NULL,
        UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),
        FOREIGN KEY(profile_id,selection_id) REFERENCES gl_cash_accounts(profile_id,id),
        FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))''')
    conn.execute('''CREATE TABLE IF NOT EXISTS gl_cash_flows (
        id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,cash_line_id TEXT NOT NULL,
        offset_line_id TEXT NOT NULL,amount_minor INTEGER NOT NULL
        CHECK(typeof(amount_minor)='integer' AND amount_minor>0),
        classification TEXT NOT NULL CHECK(classification IN ('income','expenditure','non_result')),
        evidence_id TEXT NOT NULL,locator TEXT NOT NULL,reason TEXT NOT NULL,
        idempotency_key TEXT NOT NULL,payload_digest TEXT NOT NULL,
        UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),
        FOREIGN KEY(cash_line_id) REFERENCES gl_lines(id),
        FOREIGN KEY(offset_line_id) REFERENCES gl_lines(id),
        FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))''')
    for name in ('accounts', 'counts', 'flows'):
        table = 'gl_cash_' + name
        void = table + '_voids'
        conn.execute(f'''CREATE TABLE IF NOT EXISTS {void} (
            record_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,reason TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,payload_digest TEXT NOT NULL,
            UNIQUE(profile_id,idempotency_key),
            FOREIGN KEY(profile_id,record_id) REFERENCES {table}(profile_id,id))''')
        for current, key in ((table, 'id'), (void, 'record_id')):
            for action in ('UPDATE', 'DELETE'):
                conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {current}_no_{action.lower()}
                    BEFORE {action} ON {current} BEGIN SELECT RAISE(ABORT,'accounting_cash_retained'); END''')
            conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {current}_no_replace BEFORE INSERT ON {current}
                WHEN EXISTS(SELECT 1 FROM {current} WHERE {key}=NEW.{key}
                    OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key))
                BEGIN SELECT RAISE(ABORT,'accounting_cash_retained'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_cash_account_unique BEFORE INSERT ON gl_cash_accounts
        WHEN EXISTS(SELECT 1 FROM gl_cash_accounts a WHERE a.profile_id=NEW.profile_id
            AND a.account_code=NEW.account_code AND NOT EXISTS
            (SELECT 1 FROM gl_cash_accounts_voids v WHERE v.record_id=a.id))
        OR NOT EXISTS(SELECT 1 FROM gl_accounts a WHERE a.profile_id=NEW.profile_id
            AND a.code=NEW.account_code AND (a.kind='asset' OR (NEW.role='loan' AND a.kind='liability')))
        OR EXISTS(SELECT 1 FROM gl_cash_flows f JOIN gl_lines l ON l.id=f.cash_line_id
            JOIN gl_lines o ON o.id=f.offset_line_id WHERE f.profile_id=NEW.profile_id
            AND (l.account_code=NEW.account_code OR o.account_code=NEW.account_code)
            AND NOT EXISTS(SELECT 1 FROM gl_cash_flows_voids v WHERE v.record_id=f.id))
        BEGIN SELECT RAISE(ABORT,'accounting_cash_account'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_cash_account_void_dependencies BEFORE INSERT ON gl_cash_accounts_voids
        WHEN EXISTS(SELECT 1 FROM gl_cash_counts c WHERE c.selection_id=NEW.record_id
            AND NOT EXISTS(SELECT 1 FROM gl_cash_counts_voids v WHERE v.record_id=c.id))
        OR EXISTS(SELECT 1 FROM gl_cash_flows f JOIN gl_lines l ON l.id=f.cash_line_id
            JOIN gl_lines o ON o.id=f.offset_line_id JOIN gl_cash_accounts a ON a.id=NEW.record_id
            WHERE f.profile_id=a.profile_id AND (l.account_code=a.account_code OR o.account_code=a.account_code)
            AND NOT EXISTS(SELECT 1 FROM gl_cash_flows_voids v WHERE v.record_id=f.id))
        BEGIN SELECT RAISE(ABORT,'accounting_cash_account_dependencies'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_cash_count_scope BEFORE INSERT ON gl_cash_counts
        WHEN NOT EXISTS(SELECT 1 FROM gl_cash_accounts a WHERE a.id=NEW.selection_id
            AND a.profile_id=NEW.profile_id AND a.role='cash' AND a.effective_from<=NEW.count_date
            AND NOT EXISTS(SELECT 1 FROM gl_cash_accounts_voids v WHERE v.record_id=a.id))
        OR EXISTS(SELECT 1 FROM gl_cash_counts c WHERE c.selection_id=NEW.selection_id
            AND c.count_date=NEW.count_date AND NOT EXISTS(SELECT 1 FROM gl_cash_counts_voids v WHERE v.record_id=c.id))
        BEGIN SELECT RAISE(ABORT,'accounting_cash_count_scope'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_cash_flow_scope BEFORE INSERT ON gl_cash_flows
        WHEN NOT EXISTS(SELECT 1 FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id
            JOIN gl_lines o ON o.id=NEW.offset_line_id AND o.entry_id=e.id AND o.profile_id=e.profile_id
            JOIN gl_cash_accounts a ON a.profile_id=l.profile_id AND a.account_code=l.account_code
            WHERE l.id=NEW.cash_line_id AND l.profile_id=NEW.profile_id AND e.status='posted'
            AND e.entry_kind='normal' AND a.role IN ('cash','bank') AND a.effective_from<=e.entry_date
            AND NOT EXISTS(SELECT 1 FROM gl_cash_accounts_voids v WHERE v.record_id=a.id)
            AND ((l.debit_minor>0 AND o.credit_minor>0) OR (l.credit_minor>0 AND o.debit_minor>0))
            AND (NEW.classification='non_result' OR (o.account_kind!='equity'
                AND NOT EXISTS(SELECT 1 FROM gl_cash_accounts b WHERE b.profile_id=o.profile_id
                    AND b.account_code=o.account_code AND b.effective_from<=e.entry_date
                    AND NOT EXISTS(SELECT 1 FROM gl_cash_accounts_voids v WHERE v.record_id=b.id))
                AND ((NEW.classification='income' AND l.debit_minor>0)
                    OR (NEW.classification='expenditure' AND l.credit_minor>0)))))
        BEGIN SELECT RAISE(ABORT,'accounting_cash_flow_scope'); END''')
    for column in ('cash_line_id', 'offset_line_id'):
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS gl_cash_flow_{column}_budget BEFORE INSERT ON gl_cash_flows
            WHEN NEW.amount_minor > (SELECT debit_minor+credit_minor FROM gl_lines WHERE id=NEW.{column})
                - COALESCE((SELECT SUM(f.amount_minor) FROM gl_cash_flows f
                    WHERE (f.cash_line_id=NEW.{column} OR f.offset_line_id=NEW.{column})
                    AND NOT EXISTS(SELECT 1 FROM gl_cash_flows_voids v WHERE v.record_id=f.id)),0)
            BEGIN SELECT RAISE(ABORT,'accounting_cash_allocation_exceeded'); END''')


def _active(conn, table, profile_id):
    return ledger._rows(conn, f'''SELECT r.* FROM {table} r WHERE r.profile_id=?
        AND NOT EXISTS(SELECT 1 FROM {table}_voids v WHERE v.record_id=r.id) ORDER BY r.id''', (profile_id,))


def _roles(conn, profile_id, as_of):
    return {row['account_code']: row for row in _active(conn, 'gl_cash_accounts', profile_id)
            if row['effective_from'] <= as_of}


def _open(conn, profile_id, occurred_on):
    iso_date(occurred_on)
    require_open_interval(conn, profile_id, occurred_on, occurred_on)
    if not conn.execute("SELECT 1 FROM gl_periods WHERE profile_id=? AND state='open' AND ? BETWEEN start_date AND end_date",
                        (profile_id, occurred_on)).fetchone():
        _error('Supporting record requires an open fiscal period', 'accounting_period_closed')


def _retry(conn, table, profile_id, key, payload):
    bounded_text(key, 'idempotency_key', 200)
    checksum = ledger.digest(payload)
    old = ledger._row(conn, f'SELECT * FROM {table} WHERE profile_id=? AND idempotency_key=?', (profile_id, key))
    if old and old['payload_digest'] != checksum:
        _error('Idempotency key already has different cash facts', 'accounting_idempotency_conflict')
    return old, checksum


def select_account(conn, profile_id, *, account_code, role, effective_from, reason, idempotency_key):
    _book(conn, profile_id)
    if not isinstance(role, str) or role not in ROLES:
        _error('Choose cash, bank or loan explicitly')
    bounded_text(reason, 'reason', 2000)
    bounded_text(account_code, 'account_code', 200)
    iso_date(effective_from)
    payload = dict(account_code=account_code, role=role, effective_from=effective_from, reason=reason)
    with ledger.atomic(conn):
        old, checksum = _retry(conn, 'gl_cash_accounts', profile_id, idempotency_key, payload)
        if old:
            return old
        _open(conn, profile_id, effective_from)
        account = conn.execute('SELECT kind FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, account_code)).fetchone()
        if account is None or account['kind'] not in ({'asset', 'liability'} if role == 'loan' else {'asset'}):
            _error('Cash/bank require an asset account; loans require an asset/liability account')
        if account_code in {r['account_code'] for r in _active(conn, 'gl_cash_accounts', profile_id)}:
            _error('Account already has a reviewed cash-report role')
        if any(account_code in (_line(conn, profile_id, row['cash_line_id'])['account_code'],
                                _line(conn, profile_id, row['offset_line_id'])['account_code'])
               for row in _active(conn, 'gl_cash_flows', profile_id)):
            _error('Void affected classifications before changing the payment/loan basis')
        identifier = uuid4().hex
        conn.execute('INSERT INTO gl_cash_accounts VALUES(?,?,?,?,?,?,?,?)',
            (identifier, profile_id, account_code, role, effective_from, reason, idempotency_key, checksum))
        ledger._bump(conn, profile_id)
        return ledger._row(conn, 'SELECT * FROM gl_cash_accounts WHERE id=?', (identifier,))


def retain_count(conn, profile_id, *, account_code, count_date, counted_minor, evidence_id,
                 locator, reason, idempotency_key):
    _book(conn, profile_id)
    bounded_text(account_code, 'account_code', 200)
    bounded_text(evidence_id, 'evidence_id', 200)
    ledger.strict_minor(counted_minor)  # Explicit zero is valid; None is never zero.
    iso_date(count_date)
    for key, value in (('locator', locator), ('reason', reason)):
        bounded_text(value, key, 2000)
    require_evidence(conn, profile_id, evidence_id)
    payload = dict(account_code=account_code, count_date=count_date, counted_minor=counted_minor,
                   evidence_id=evidence_id, locator=locator, reason=reason)
    with ledger.atomic(conn):
        old, checksum = _retry(conn, 'gl_cash_counts', profile_id, idempotency_key, payload)
        if old:
            return old
        _open(conn, profile_id, count_date)
        selected = _roles(conn, profile_id, count_date).get(account_code)
        if not selected or selected['role'] != 'cash':
            _error('Choose a physical cash account before retaining its count')
        if any(c['selection_id'] == selected['id'] and c['count_date'] == count_date for c in _active(conn, 'gl_cash_counts', profile_id)):
            _error('Void the existing date count before retaining a correction')
        identifier = uuid4().hex
        conn.execute('INSERT INTO gl_cash_counts VALUES(?,?,?,?,?,?,?,?,?,?)',
            (identifier, profile_id, selected['id'], count_date, counted_minor, evidence_id, locator, reason, idempotency_key, checksum))
        ledger._bump(conn, profile_id)
        return ledger._row(conn, 'SELECT * FROM gl_cash_counts WHERE id=?', (identifier,))


def _line(conn, profile_id, line_id):
    bounded_text(line_id, 'line_id', 200)
    result = ledger._row(conn, '''SELECT l.*,e.entry_date,e.status,e.entry_kind FROM gl_lines l
        JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE l.profile_id=? AND l.id=?''', (profile_id, line_id))
    if result is None:
        _error('Posted line does not belong to this book', 'not_found')
    return result


def classify_flow(conn, profile_id, *, cash_line_id, offset_line_id, amount_minor, classification,
                  evidence_id, locator, reason, idempotency_key):
    _book(conn, profile_id)
    for key, value in (('cash_line_id', cash_line_id), ('offset_line_id', offset_line_id), ('evidence_id', evidence_id)):
        bounded_text(value, key, 200)
    ledger.strict_minor(amount_minor)
    if not amount_minor or not isinstance(classification, str) or classification not in CLASSIFICATIONS:
        _error('Choose an exact positive allocation and a reviewed classification')
    for key, value in (('locator', locator), ('reason', reason)):
        bounded_text(value, key, 2000)
    require_evidence(conn, profile_id, evidence_id)
    payload = dict(cash_line_id=cash_line_id, offset_line_id=offset_line_id, amount_minor=amount_minor,
        classification=classification, evidence_id=evidence_id, locator=locator, reason=reason)
    with ledger.atomic(conn):
        old, checksum = _retry(conn, 'gl_cash_flows', profile_id, idempotency_key, payload)
        if old:
            return old
        line, offset = (_line(conn, profile_id, identity) for identity in (cash_line_id, offset_line_id))
        if line['status'] != 'posted' or line['entry_kind'] != 'normal' or line['entry_id'] != offset['entry_id']:
            _error('Classify opposite lines of the same ordinary posted entry; reversals inherit their original')
        if (line['debit_minor'] > 0) == (offset['debit_minor'] > 0):
            _error('Allocation requires opposite debit/credit sides')
        _open(conn, profile_id, line['entry_date'])
        roles = _roles(conn, profile_id, line['entry_date'])
        selected = roles.get(line['account_code'])
        if not selected or selected['role'] not in {'cash', 'bank'}:
            _error('Select the payment account explicitly before classification')
        if classification != 'non_result':
            if offset['account_code'] in roles or offset['account_kind'] == 'equity':
                _error('Internal transfers, selected loans and equity are not income/expenditure')
            if (classification == 'income') != (line['debit_minor'] > 0):
                _error('Income requires a receipt; expenditure requires a payment')
        active = _active(conn, 'gl_cash_flows', profile_id)
        for endpoint in (line, offset):
            used = sum(row['amount_minor'] for row in active if endpoint['id'] in (row['cash_line_id'], row['offset_line_id']))
            if amount_minor > endpoint['debit_minor'] + endpoint['credit_minor'] - used:
                _error('Classification exceeds the unallocated posted line', 'accounting_cash_allocation_exceeded')
        identifier = uuid4().hex
        conn.execute('INSERT INTO gl_cash_flows VALUES(?,?,?,?,?,?,?,?,?,?,?)',
            (identifier, profile_id, cash_line_id, offset_line_id, amount_minor, classification,
             evidence_id, locator, reason, idempotency_key, checksum))
        ledger._bump(conn, profile_id)
        return ledger._row(conn, 'SELECT * FROM gl_cash_flows WHERE id=?', (identifier,))


def void_record(conn, profile_id, *, record_kind, record_id, reason, idempotency_key):
    _book(conn, profile_id)
    bounded_text(record_id, 'record_id', 200)
    if not isinstance(record_kind, str) or record_kind not in ('accounts', 'counts', 'flows'):
        _error('Unknown cash record type')
    bounded_text(reason, 'reason', 2000)
    table = 'gl_cash_' + record_kind
    with ledger.atomic(conn):
        old, checksum = _retry(conn, table + '_voids', profile_id, idempotency_key, dict(record_id=record_id, reason=reason))
        if old:
            return old
        record = ledger._row(conn, f'SELECT * FROM {table} WHERE id=? AND profile_id=?', (record_id, profile_id))
        if not record:
            _error('Cash record not found in this book', 'not_found')
        date = record.get('effective_from') or record.get('count_date') or _line(conn, profile_id, record['cash_line_id'])['entry_date']
        _open(conn, profile_id, date)
        if record_kind == 'accounts':
            if any(row['selection_id'] == record_id for row in _active(conn, 'gl_cash_counts', profile_id)) or any(
                record['account_code'] in (_line(conn, profile_id, row['cash_line_id'])['account_code'],
                                          _line(conn, profile_id, row['offset_line_id'])['account_code'])
                for row in _active(conn, 'gl_cash_flows', profile_id)):
                _error('Void dependent counts/classifications before changing the account role')
        if conn.execute(f'SELECT 1 FROM {table}_voids WHERE record_id=?', (record_id,)).fetchone():
            _error('Cash record already voided')
        conn.execute(f'INSERT INTO {table}_voids VALUES(?,?,?,?,?)', (record_id, profile_id, reason, idempotency_key, checksum))
        ledger._bump(conn, profile_id)
        return dict(record_id=record_id, record_kind=record_kind, voided=True)


def reconciliation(conn, profile_id, *, as_of):
    book = _book(conn, profile_id)
    iso_date(as_of)
    roles = _roles(conn, profile_id, as_of)
    balances = {row['account_code']: row['balance_minor'] for row in ledger.trial_balance(conn, profile_id, as_of=as_of)['rows']}
    counts = _active(conn, 'gl_cash_counts', profile_id)
    rows = []
    for code, selection in sorted(roles.items()):
        if selection['role'] != 'cash':
            continue
        current = next((c for c in counts if c['selection_id'] == selection['id'] and c['count_date'] == as_of), None)
        previous = max((c['count_date'] for c in counts if c['selection_id'] == selection['id'] and c['count_date'] <= as_of), default=None)
        balance = balances.get(code, 0)
        difference = current['counted_minor'] - balance if current else None
        rows.append(dict(account_code=code, selection_id=selection['id'], posted_minor=balance,
            counted_minor=current['counted_minor'] if current else None, difference_minor=difference,
            count_id=current['id'] if current else None, latest_count_date=previous,
            status='missing_count' if current is None else 'negative_ledger_cash' if balance < 0 else 'matched' if difference == 0 else 'count_mismatch'))
    return dict(as_of=as_of, configured=bool(rows), rows=rows, complete=all(row['status'] == 'matched' for row in rows),
        currency=book['currency'], minor_unit_exponent=book['minor_unit_exponent'], revision=book['revision'])


def report(conn, profile_id, *, start_date, end_date):
    book = _book(conn, profile_id)
    iso_date(start_date)
    iso_date(end_date)
    if start_date > end_date:
        _error('Invalid cash report interval')
    selections = _active(conn, 'gl_cash_accounts', profile_id)
    roles = {row['account_code']: row for row in selections if row['effective_from'] <= end_date}
    liquidity = {code for code, row in roles.items() if row['role'] in {'cash', 'bank'}}
    lines = ledger._rows(conn, '''SELECT l.*,e.entry_date,e.entry_kind,e.reversal_of FROM gl_lines l
        JOIN gl_entries e ON e.id=l.entry_id AND e.profile_id=l.profile_id
        WHERE l.profile_id=? AND e.status='posted' AND e.entry_date<=? ORDER BY e.entry_date,e.id,l.position''', (profile_id, end_date))
    by_id = {row['id']: row for row in lines}
    entry_kinds = {row['entry_id']: row['entry_kind'] for row in lines}
    inverse = {(row['reversal_of'], row['position']): row for row in lines if row['reversal_of']}
    covered, rows = {}, []
    for allocation in _active(conn, 'gl_cash_flows', profile_id):
        original = by_id.get(allocation['cash_line_id'])
        if original is None:
            continue
        for identity in (allocation['cash_line_id'], allocation['offset_line_id']):
            source = by_id[identity]
            if source['account_code'] not in liquidity or source['entry_date'] < roles[source['account_code']]['effective_from']:
                continue
            for line, is_reversal in ((source, False), (inverse.get((source['entry_id'], source['position'])), True)):
                if line is None:
                    continue
                covered[line['id']] = covered.get(line['id'], 0) + allocation['amount_minor']
                if not start_date <= line['entry_date'] <= end_date:
                    continue
                sign = 1 if line['debit_minor'] else -1
                rows.append(dict(allocation_id=allocation['id'], line_id=line['id'], entry_id=line['entry_id'],
                    account_code=line['account_code'], occurred_on=line['entry_date'], classification=allocation['classification'],
                    amount_minor=sign * allocation['amount_minor'], reversal=is_reversal,
                    evidence_id=allocation['evidence_id'], locator=allocation['locator'], reason=allocation['reason']))
    missing, coverage_gaps = [], []
    for line in lines:
        if line['account_code'] not in liquidity or not start_date <= line['entry_date'] <= end_date:
            continue
        if line['entry_kind'] in {'opening', 'closing'} or entry_kinds.get(line['reversal_of']) in {'opening', 'closing'}:
            continue
        # A later role selection must not silently erase ordinary payments
        # within this report interval. Do not classify them retroactively: the
        # user must review the account's effective date/coverage first. Opening
        # balances and activity before the requested interval remain exempt.
        if line['entry_date'] < roles[line['account_code']]['effective_from']:
            coverage_gaps.append(dict(line_id=line['id'], entry_id=line['entry_id'], account_code=line['account_code'],
                occurred_on=line['entry_date'], effective_from=roles[line['account_code']]['effective_from']))
            continue
        remaining = line['debit_minor'] + line['credit_minor'] - covered.get(line['id'], 0)
        if remaining:
            missing.append(dict(line_id=line['id'], entry_id=line['entry_id'], account_code=line['account_code'],
                occurred_on=line['entry_date'], debit_minor=line['debit_minor'], credit_minor=line['credit_minor'],
                remaining_minor=remaining, entry_kind=line['entry_kind']))
    configured = bool(liquidity)
    income = sum(row['amount_minor'] for row in rows if row['classification'] == 'income')
    expenditure = -sum(row['amount_minor'] for row in rows if row['classification'] == 'expenditure')
    return dict(start_date=start_date, end_date=end_date, configured=configured, complete=configured and not missing and not coverage_gaps,
        basis='reviewed_posted_payment_allocations_not_accrual_profit', accounting_regime=book['accounting_regime'],
        selections=list(roles.values()), income_minor=income if configured else None, expenditure_minor=expenditure if configured else None,
        surplus_minor=income - expenditure if configured else None, rows=rows, unclassified=missing, coverage_gaps=coverage_gaps,
        currency=book['currency'], minor_unit_exponent=book['minor_unit_exponent'], revision=book['revision'])


def asset_statement(conn, profile_id, *, as_of):
    book = _book(conn, profile_id)
    iso_date(as_of)
    balance = ledger.trial_balance(conn, profile_id, as_of=as_of)
    assets = [row for row in balance['rows'] if row['kind'] == 'asset']
    liabilities = [dict(row, carrying_minor=-row['balance_minor']) for row in balance['rows'] if row['kind'] == 'liability']
    assets_minor = sum(row['balance_minor'] for row in assets)
    liabilities_minor = sum(row['carrying_minor'] for row in liabilities)
    return dict(as_of=as_of, basis='posted_general_ledger_carrying_values', accounting_regime=book['accounting_regime'],
        assets=assets, liabilities=liabilities, assets_minor=assets_minor, liabilities_minor=liabilities_minor,
        net_assets_minor=assets_minor - liabilities_minor, currency=book['currency'],
        minor_unit_exponent=book['minor_unit_exponent'], revision=book['revision'])


def snapshot(conn, profile_id, *, start_date, end_date):
    return dict(report=report(conn, profile_id, start_date=start_date, end_date=end_date),
        reconciliation=reconciliation(conn, profile_id, as_of=end_date),
        asset_statement=asset_statement(conn, profile_id, as_of=end_date),
        counts=_active(conn, 'gl_cash_counts', profile_id), flows=_active(conn, 'gl_cash_flows', profile_id))


def validate_close(conn, profile_id, start_date, end_date):
    """Pure close inputs. Optional accrual books need no invented cash basis."""
    book = _book(conn, profile_id)
    payments = report(conn, profile_id, start_date=start_date, end_date=end_date)
    counts = reconciliation(conn, profile_id, as_of=end_date)
    assets = asset_statement(conn, profile_id, as_of=end_date)
    required = book['accounting_regime'] == 'cash_basis'
    blockers = []
    for row in counts['rows']:
        if row['status'] != 'matched':
            blockers.append(dict(code='accounting_cash_' + row['status'], account_code=row['account_code']))
    if required and not payments['configured']:
        blockers.append(dict(code='accounting_cash_basis_missing'))
    if required and payments['unclassified']:
        blockers.append(dict(code='accounting_cash_flows_unclassified', count=len(payments['unclassified'])))
    if required and payments['coverage_gaps']:
        blockers.append(dict(code='accounting_cash_selection_gap', count=len(payments['coverage_gaps'])))
    return dict(required=required, configured=payments['configured'], report=payments,
        reconciliation=counts, asset_statement=assets, blockers=blockers, complete=not blockers)
