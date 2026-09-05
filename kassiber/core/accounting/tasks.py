"""Durable local period tasks composed from the existing accounting services.

An assignment rule authorizes preparation only. Every financial transition has
its own exact preview and explicit approval; task state never grants AI access.
"""
from __future__ import annotations

import json
from uuid import uuid4

from ...errors import AppError
from . import bank, ledger, posting_batch
from .task_schema import ensure_schema

READ_ACTIONS = frozenset({'task-list', 'task-get', 'task-preview', 'task-amend-preview', 'rule-list'})
WRITE_ACTIONS = frozenset({'task-create', 'task-apply', 'task-cancel', 'task-amend', 'task-source-assign', 'rule-create', 'rule-revoke'})
STEPS = frozenset({'prepare', 'post', 'close', 'tax_finalize', 'export_close', 'export_tax'})
MAX_SOURCES = 10000


def _fail(code='accounting_task_invalid'):
    raise AppError('Accounting task requires current, explicitly reviewed inputs', code=code)


def _fields(p, allowed, required=()):
    if not isinstance(p, dict) or set(p) - set(allowed) or set(required) - set(p):
        _fail()


def _ids(value):
    if not isinstance(value, list) or len(value) > MAX_SOURCES or any(
            not isinstance(x, str) or not x or len(x) > 128 for x in value) or len(set(value)) != len(value):
        _fail()
    return sorted(value)


def _task(conn, profile_id, task_id):
    ledger.require_book(conn, profile_id)
    ledger._text(task_id, 'task_id', maximum=128)
    row = ledger._row(conn, 'SELECT * FROM gl_accounting_tasks WHERE profile_id=? AND id=?', (profile_id, task_id))
    if not row:
        _fail('accounting_task_not_found')
    row['spec'] = json.loads(row.pop('spec_json'))
    # The original selection and every prior receipt remain immutable. Only a
    # separately reviewed, additive local receipt expands the effective scope.
    amendments = ledger._rows(conn, "SELECT result_json FROM gl_accounting_task_receipts WHERE profile_id=? AND task_id=? AND step='amend_sources' ORDER BY rowid", (profile_id, task_id))
    for amendment in amendments:
        row['spec']['evidence_ids'].extend(json.loads(amendment['result_json'])['evidence_ids'])
    row['spec']['evidence_ids'].sort()
    row['scope_revision'] = len(amendments)
    return row


def _amend_preview(conn, profile_id, p):
    keys = {'task_id', 'period_id', 'evidence_ids', 'reason'}
    _fields(p, keys, keys)
    ledger._text(p['reason'], 'reason', maximum=2000)
    ledger._text(p['period_id'], 'period_id', maximum=128)
    task = _task(conn, profile_id, p['task_id'])
    if p['period_id'] != task['period_id']:
        _fail('accounting_task_source_scope')
    current = get(conn, profile_id, task['id'])
    if current['state'] == 'cancelled':
        _fail('accounting_task_cancelled')
    period = ledger._period(conn, profile_id, task['period_id'])
    if period['state'] != 'open':
        _fail('accounting_task_blocked')
    identifiers = _ids(p['evidence_ids'])
    if not identifiers or set(identifiers) & set(task['spec']['evidence_ids']):
        _fail('accounting_task_source_scope')
    if current['source_count'] + len(identifiers) > MAX_SOURCES:
        _fail('accounting_task_population_limit')
    from .evidence import require_evidence
    evidence = [require_evidence(conn, profile_id, i) for i in identifiers]
    book = ledger.require_book(conn, profile_id)
    journal = ledger._row(conn, 'SELECT journal_input_version FROM profiles WHERE id=?', (profile_id,))
    binding = dict(profile_id=profile_id, book=book, period=period, task=current,
        evidence=evidence, reason=p['reason'], journal_input_version=journal['journal_input_version'])
    return dict(task_id=task['id'], period_id=task['period_id'], scope_revision=task['scope_revision'],
        evidence_ids=identifiers, evidence=evidence, reason=p['reason'],
        expected_revision=book['revision'], expected_digest=ledger.digest(binding))


def _amend(conn, profile_id, p):
    selection = {'task_id', 'period_id', 'evidence_ids', 'reason'}
    keys = selection | {'expected_digest', 'expected_revision', 'idempotency_key', 'confirmed'}
    _fields(p, keys, keys)
    if p['confirmed'] is not True:
        _fail('accounting_task_consent_required')
    ledger._text(p['idempotency_key'], 'idempotency_key', maximum=128)
    _task(conn, profile_id, p['task_id'])
    request_digest = ledger.digest(p)
    prior = ledger._row(conn, 'SELECT * FROM gl_accounting_task_receipts WHERE profile_id=? AND idempotency_key=?', (profile_id, p['idempotency_key']))
    # Retry before checking the now-expanded scope, including after completion.
    if prior:
        if prior['request_digest'] != request_digest or prior['step'] != 'amend_sources':
            _fail('accounting_idempotency_conflict')
        retained = json.loads(prior['result_json'])
        return dict(task=get(conn, profile_id, p['task_id']), receipt={'id': prior['id'], 'step': prior['step'], 'result': retained}, already_applied=True)
    reviewed = _amend_preview(conn, profile_id, {key: p[key] for key in selection})
    if type(p['expected_revision']) is not int or reviewed['expected_revision'] != p['expected_revision'] or reviewed['expected_digest'] != p['expected_digest']:
        _fail('accounting_stale_approval')
    retained = dict(period_id=p['period_id'], evidence_ids=reviewed['evidence_ids'], reason=p['reason'],
        previous_scope_revision=reviewed['scope_revision'], scope_revision=reviewed['scope_revision'] + 1,
        expected_revision=p['expected_revision'], expected_digest=p['expected_digest'],
        evidence_digests={r['id']: r['content_sha256'] for r in reviewed['evidence']})
    identifier = uuid4().hex
    conn.execute('INSERT INTO gl_accounting_task_receipts(id,profile_id,task_id,step,idempotency_key,request_digest,result_json) VALUES(?,?,?,?,?,?,?)',
        (identifier, profile_id, p['task_id'], 'amend_sources', p['idempotency_key'], request_digest, ledger.canonical_json(retained)))
    ledger._bump(conn, profile_id)
    return dict(task=get(conn, profile_id, p['task_id']), receipt={'id': identifier, 'step': 'amend_sources', 'result': retained}, already_applied=False)


def _rules(conn, profile_id):
    ledger.require_book(conn, profile_id)
    return [dict(id=r['id'], **json.loads(r['payload_json']), revoked=bool(r['revoked'])) for r in conn.execute(
        '''SELECT r.*,EXISTS(SELECT 1 FROM gl_accounting_task_rule_revocations v WHERE v.rule_id=r.id) AS revoked
           FROM gl_accounting_task_rules r WHERE profile_id=? ORDER BY id''', (profile_id,))]


def _create_rule(conn, profile_id, p):
    keys = {'idempotency_key', 'account_code', 'direction', 'description_exact', 'counter_account_code', 'reason', 'confirmed'}
    _fields(p, keys, keys)
    book = ledger.require_book(conn, profile_id)
    if p['confirmed'] is not True or p['direction'] not in ('in', 'out') or p['account_code'] == p['counter_account_code']:
        _fail('accounting_task_consent_required')
    for key in keys - {'confirmed', 'direction'}:
        ledger._text(p[key], key, maximum=2000 if key in ('reason', 'description_exact') else 128)
    for key in ('account_code', 'counter_account_code'):
        account = ledger._row(conn, 'SELECT kind FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, p[key]))
        if not account or (key == 'account_code' and account['kind'] != 'asset'):
            _fail()
    payload = {**p, 'currency': book['currency'], 'minor_unit_exponent': book['minor_unit_exponent']}
    digest = ledger.digest(payload)
    prior = ledger._row(conn, 'SELECT * FROM gl_accounting_task_rules WHERE profile_id=? AND idempotency_key=?', (profile_id, p['idempotency_key']))
    if prior:
        if prior['request_digest'] != digest:
            _fail('accounting_idempotency_conflict')
        return {'id': prior['id'], **payload}
    identifier = uuid4().hex
    conn.execute('INSERT INTO gl_accounting_task_rules VALUES(?,?,?,?,?)',
                 (identifier, profile_id, p['idempotency_key'], ledger.canonical_json(payload), digest))
    return {'id': identifier, **payload}


def _create(conn, profile_id, p):
    _fields(p, {'period_id', 'idempotency_key', 'statement_ids', 'include_period_statements', 'evidence_ids', 'draft_ids', 'tax_workpaper_id'},
            {'period_id', 'idempotency_key'})
    ledger._text(p['idempotency_key'], 'idempotency_key', maximum=128)
    ledger._text(p['period_id'], 'period_id', maximum=128)
    if ('statement_ids' in p) == ('include_period_statements' in p) or ('include_period_statements' in p and p['include_period_statements'] is not True):
        _fail()
    prior = ledger._row(conn, 'SELECT id,request_digest FROM gl_accounting_tasks WHERE profile_id=? AND idempotency_key=?', (profile_id, p['idempotency_key']))
    if prior:
        if prior['request_digest'] != ledger.digest(p):
            _fail('accounting_idempotency_conflict')
        return get(conn, profile_id, prior['id'])
    period = ledger._period(conn, profile_id, p['period_id'])
    statements = _ids(p.get('statement_ids', [r[0] for r in conn.execute(
        '''SELECT id FROM gl_bank_statements s WHERE profile_id=? AND start_date<=? AND end_date>=?
           AND NOT EXISTS(SELECT 1 FROM gl_bank_statement_voids v WHERE v.statement_id=s.id)''',
        (profile_id, period['end_date'], period['start_date']))]))
    for identifier in statements:
        row = ledger._row(conn, 'SELECT * FROM gl_bank_statements WHERE profile_id=? AND id=?', (profile_id, identifier))
        if not row or row['start_date'] > period['end_date'] or row['end_date'] < period['start_date']:
            _fail('accounting_task_source_scope')
    evidence_ids, draft_ids = _ids(p.get('evidence_ids', [])), _ids(p.get('draft_ids', []))
    from .evidence import require_evidence
    for identifier in evidence_ids:
        require_evidence(conn, profile_id, identifier)
    for identifier in draft_ids:
        if ledger._entry(conn, profile_id, identifier)['period_id'] != p['period_id']:
            _fail('accounting_task_source_scope')
    if p.get('tax_workpaper_id'):
        from .tax_workpapers import get_workpaper
        if get_workpaper(conn, profile_id, workpaper_id=p['tax_workpaper_id'])['period_id'] != p['period_id']:
            _fail('accounting_task_source_scope')
    spec = dict(statement_ids=statements, evidence_ids=evidence_ids, draft_ids=draft_ids,
                tax_workpaper_id=p.get('tax_workpaper_id'))
    identifier = uuid4().hex
    conn.execute('INSERT INTO gl_accounting_tasks(id,profile_id,period_id,idempotency_key,spec_json,request_digest) VALUES(?,?,?,?,?,?)',
                 (identifier, profile_id, p['period_id'], p['idempotency_key'], ledger.canonical_json(spec), ledger.digest(p)))
    result = get(conn, profile_id, identifier)
    if result['source_count'] > MAX_SOURCES:
        _fail('accounting_task_size')
    return result


def _population(conn, profile_id, task):
    period = ledger._period(conn, profile_id, task['period_id'])
    book = ledger.require_book(conn, profile_id)
    rules = [r for r in _rules(conn, profile_id) if not r['revoked']]
    coverage, proposals, represented = [], [], set()
    for statement_id in task['spec']['statement_ids']:
        report = bank.reconcile_statement(conn, profile_id, statement_id)
        statement = report['statement']
        if statement['evidence_id']:
            represented.add(statement['evidence_id'])
        for row in report['rows']:
            if not period['start_date'] <= row['occurred_on'] <= period['end_date']:
                continue
            item = dict(source_kind='bank', source_id=row['id'], status='exception',
                statement_id=statement_id,
                account_code=statement['account_code'], occurred_on=row['occurred_on'],
                amount_minor=row['amount_minor'], description=row['description'])
            claim = ledger._row(conn, 'SELECT entry_id FROM gl_accounting_task_claims WHERE profile_id=? AND source_kind=? AND source_id=?',
                                (profile_id, 'bank', row['id']))
            if 'statement_voided' in report['blockers']:
                item['exception'] = 'statement_voided'
            elif claim:
                entry = ledger._entry(conn, profile_id, claim['entry_id'])
                item.update(entry_id=entry['id'], status=entry['status'])
                if entry['status'] == 'discarded':
                    item.update(status='exception', exception='prepared_draft_discarded')
                if entry['status'] == 'posted' and row['remaining_minor']:
                    item.update(status='exception', exception='bank_allocation_changed')
            elif row['remaining_minor'] == 0:
                item['status'] = 'covered'
            elif row['remaining_minor'] != abs(row['amount_minor']):
                item['exception'] = 'partial_bank_allocation'
            elif conn.execute('''SELECT 1 FROM gl_lines l JOIN gl_entries e ON e.id=l.entry_id
                WHERE e.profile_id=? AND e.entry_date=? AND e.status IN ('draft','posted') AND l.account_code=?
                AND l.debit_minor-l.credit_minor=?
                AND abs(l.debit_minor-l.credit_minor) > COALESCE((SELECT SUM(a.amount_minor)
                    FROM gl_bank_allocations a WHERE a.profile_id=e.profile_id AND a.line_id=l.id
                    AND NOT EXISTS(SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)),0)''',
                (profile_id, row['occurred_on'], statement['account_code'], row['amount_minor'])).fetchone():
                item['exception'] = 'possible_existing_entry'
            else:
                candidates = [r for r in rules if r['account_code'] == statement['account_code']
                    and r['description_exact'] == row['description'] and r['direction'] == ('in' if row['amount_minor'] > 0 else 'out')
                    and r['currency'] == book['currency'] and r['minor_unit_exponent'] == book['minor_unit_exponent']]
                if len(candidates) != 1:
                    item['exception'] = 'missing_assignment_rule' if not candidates else 'ambiguous_assignment_rule'
                else:
                    rule = candidates[0]
                    amount = abs(row['amount_minor'])
                    debit, credit = (statement['account_code'], rule['counter_account_code']) if row['amount_minor'] > 0 else (rule['counter_account_code'], statement['account_code'])
                    payload = dict(idempotency_key='task-bank-' + row['id'], period_id=task['period_id'], entry_date=row['occurred_on'],
                        description=row['description'] or 'Reviewed bank assignment', source_ref='bank-row:' + row['id'],
                        lines=[dict(account_code=debit, debit_minor=amount, credit_minor=0), dict(account_code=credit, debit_minor=0, credit_minor=amount)])
                    item.update(status='ready', rule_id=rule['id'])
                    proposals.append(dict(source_kind='bank', source_id=row['id'], rule_id=rule['id'], payload=payload))
            coverage.append(item)
    from .evidence import require_evidence
    seen_evidence = {}
    for identifier in task['spec']['evidence_ids']:
        metadata = require_evidence(conn, profile_id, identifier)
        checksum = metadata['content_sha256']
        item = dict(source_kind='evidence', source_id=identifier, status='exception', content_digest=checksum, name=metadata['name'])
        assignment_row = ledger._row(conn, '''SELECT a.* FROM gl_accounting_task_evidence_assignments a
            JOIN gl_evidence e ON e.id=a.evidence_id WHERE a.profile_id=? AND e.content_sha256=?
            AND NOT EXISTS(SELECT 1 FROM gl_accounting_task_evidence_assignments n WHERE n.previous_id=a.id)
            ORDER BY a.id''', (profile_id, checksum))
        assignment = json.loads(assignment_row['payload_json']) if assignment_row else None
        item['assignment_id'] = assignment_row['id'] if assignment_row else None
        # Desktop-only choices, not raw extracted document text. The selected
        # review is integrity-checked again by _evidence_payload before use.
        reviews = ledger._rows(conn, '''SELECT x.id AS extraction_id,r.content_digest AS review_digest,r.fields_json
            FROM gl_evidence_extractions x JOIN gl_evidence_field_reviews r ON r.extraction_id=x.id
            WHERE x.profile_id=? AND x.evidence_id=? AND NOT EXISTS
            (SELECT 1 FROM gl_evidence_field_reviews n WHERE n.previous_id=r.id)
            ORDER BY x.created_at DESC,x.id DESC LIMIT 21''', (profile_id, identifier))
        item['review_options'] = [dict(extraction_id=r['extraction_id'], review_digest=r['review_digest'],
            fields={k: v for k, v in json.loads(r['fields_json']).items() if k in
                    ('total_minor', 'vat_minor', 'issued_date', 'currency', 'minor_unit_exponent')}) for r in reviews[:20]]
        item['reviews_truncated'] = len(reviews) > 20
        claim = ledger._row(conn, 'SELECT entry_id FROM gl_accounting_task_claims WHERE profile_id=? AND source_kind=? AND source_id=?',
                            (profile_id, 'evidence', checksum))
        if identifier in represented:
            item['status'] = 'covered'
        elif claim:
            entry = ledger._entry(conn, profile_id, claim['entry_id'])
            item.update(entry_id=entry['id'], status=entry['status'])
            if entry['status'] == 'discarded':
                item.update(status='exception', exception='prepared_draft_discarded')
            if assignment and assignment['kind'] == 'evidence_posting':
                try:
                    payload, review = _evidence_payload(conn, profile_id, task, assignment)
                    item['review_digest'] = review['content_digest']
                    # Re-review may confirm the same exact draft, but never silently
                    # reinterpret an already-created monetary entry.
                    normalized = ledger._normalize_payload(conn, profile_id, payload)
                    if ledger.digest(normalized) != entry['payload_digest']:
                        item.update(status='exception', exception='prepared_evidence_changed')
                except AppError as exc:
                    item.update(status='exception', exception=exc.code)
        elif assignment and assignment['kind'] == 'bank_evidence':
            target = next((r for r in coverage if r['source_kind'] == 'bank' and r['source_id'] == assignment['bank_row_id']), None)
            if target and target['status'] in ('covered', 'posted', 'draft', 'ready'):
                item.update(status='covered', bank_row_id=assignment['bank_row_id'])
            else:
                item['exception'] = 'linked_bank_row_unresolved'
        elif assignment:
            try:
                payload, review = _evidence_payload(conn, profile_id, task, assignment)
                item.update(status='ready', review_digest=review['content_digest'])
                if checksum not in seen_evidence:
                    proposals.append(dict(source_kind='evidence', source_id=checksum, evidence_id=identifier, payload=payload))
            except AppError as exc:
                item['exception'] = exc.code
        else:
            item['exception'] = 'evidence_assignment_required'
        if checksum in seen_evidence:
            item['duplicate_of'] = seen_evidence[checksum]
        seen_evidence[checksum] = identifier
        coverage.append(item)
    for identifier in task['spec']['draft_ids']:
        entry = ledger._entry(conn, profile_id, identifier)
        coverage.append(dict(source_kind='draft', source_id=identifier, entry_id=identifier,
            status=entry['status'] if entry['status'] in ('draft', 'posted') else 'exception',
            **({'exception': 'selected_draft_discarded'} if entry['status'] == 'discarded' else {})))
    for item in coverage:
        if item.get('entry_id') and conn.execute("SELECT 1 FROM gl_entries WHERE profile_id=? AND reversal_of=? AND status='posted'",
                                               (profile_id, item['entry_id'])).fetchone():
            item.update(status='exception', exception='source_entry_reversed')
    return coverage, proposals


def _evidence_payload(conn, profile_id, task, assignment):
    from . import document_text
    book = ledger.require_book(conn, profile_id)
    extraction = document_text.get(conn, profile_id, extraction_id=assignment['extraction_id'])
    if extraction['evidence_id'] != assignment['evidence_id']:
        _fail('accounting_task_source_scope')
    review = extraction['review']
    if not review or review['content_digest'] != assignment['expected_review_digest']:
        _fail('accounting_document_review_changed')
    fields = review['fields']
    if not {'total_minor', 'vat_minor', 'issued_date', 'currency', 'minor_unit_exponent'} <= set(fields):
        _fail('accounting_document_missing_facts')
    if fields['currency'] != book['currency'] or fields['minor_unit_exponent'] != book['minor_unit_exponent']:
        _fail('accounting_document_currency_mismatch')
    if fields.get('vat_minor', 0) != 0 or fields['total_minor'] <= 0:
        _fail('accounting_document_split_required')
    period = ledger._period(conn, profile_id, task['period_id'])
    if not period['start_date'] <= fields['issued_date'] <= period['end_date']:
        _fail('accounting_task_source_scope')
    checksum = extraction['source_digest']
    payload = dict(idempotency_key='task-evidence-' + checksum, period_id=task['period_id'], entry_date=fields['issued_date'],
        description='Reviewed document assignment', source_ref='evidence:' + assignment['evidence_id'],
        lines=[dict(account_code=assignment['debit_account_code'], debit_minor=fields['total_minor'], credit_minor=0),
               dict(account_code=assignment['credit_account_code'], debit_minor=0, credit_minor=fields['total_minor'])])
    ledger._normalize_payload(conn, profile_id, payload)
    return payload, review


def _assign_source(conn, profile_id, p):
    common = {'task_id', 'evidence_id', 'kind', 'reason', 'confirmed', 'idempotency_key'}
    required = {'bank_row_id'} if p.get('kind') == 'bank_evidence' else {'extraction_id', 'expected_review_digest', 'debit_account_code', 'credit_account_code'}
    _fields(p, common | required | {'previous_id'}, common | required)
    if p['confirmed'] is not True or p['kind'] not in ('bank_evidence', 'evidence_posting'):
        _fail('accounting_task_consent_required')
    for key in set(p) - {'confirmed', 'previous_id'}:
        ledger._text(p[key], key, maximum=2000 if key == 'reason' else 128)
    if p.get('previous_id') is not None:
        ledger._text(p['previous_id'], 'previous_id', maximum=128)
    task = _task(conn, profile_id, p['task_id'])
    if get(conn, profile_id, task['id'])['state'] == 'cancelled':
        _fail('accounting_task_cancelled')
    if p['evidence_id'] not in task['spec']['evidence_ids']:
        _fail('accounting_task_source_scope')
    digest = ledger.digest(p)
    prior = ledger._row(conn, 'SELECT * FROM gl_accounting_task_evidence_assignments WHERE profile_id=? AND idempotency_key=?', (profile_id, p['idempotency_key']))
    if prior:
        if prior['request_digest'] != digest:
            _fail('accounting_idempotency_conflict')
        return {'id': prior['id'], 'task': get(conn, profile_id, task['id'])}
    from .evidence import require_evidence
    checksum = require_evidence(conn, profile_id, p['evidence_id'])['content_sha256']
    latest = ledger._row(conn, '''SELECT a.id FROM gl_accounting_task_evidence_assignments a JOIN gl_evidence e ON e.id=a.evidence_id
        WHERE a.profile_id=? AND e.content_sha256=? AND NOT EXISTS
        (SELECT 1 FROM gl_accounting_task_evidence_assignments n WHERE n.previous_id=a.id)''', (profile_id, checksum))
    if (latest['id'] if latest else None) != p.get('previous_id'):
        _fail('accounting_stale_approval')
    if latest and conn.execute("SELECT 1 FROM gl_accounting_task_claims WHERE profile_id=? AND source_kind='evidence' AND source_id=?", (profile_id, checksum)).fetchone():
        # An existing source posting cannot be turned into a bank alias. Correct
        # its ledger interpretation explicitly before changing its source role.
        prior_payload = json.loads(ledger._row(conn, 'SELECT payload_json FROM gl_accounting_task_evidence_assignments WHERE id=?', (latest['id'],))['payload_json'])
        if any(prior_payload.get(k) != p.get(k) for k in ('kind', 'debit_account_code', 'credit_account_code', 'extraction_id')):
            _fail('accounting_task_source_already_posted')
    if p['kind'] == 'bank_evidence':
        if not any(r['source_kind'] == 'bank' and r['source_id'] == p['bank_row_id'] for r in get(conn, profile_id, task['id'])['coverage']):
            _fail('accounting_task_source_scope')
    else:
        _evidence_payload(conn, profile_id, task, p)
    identifier = uuid4().hex
    conn.execute('INSERT INTO gl_accounting_task_evidence_assignments VALUES(?,?,?,?,?,?,?,?)',
        (identifier, profile_id, task['id'], p['evidence_id'], p['idempotency_key'], ledger.canonical_json(p), digest, p.get('previous_id')))
    return {'id': identifier, 'task': get(conn, profile_id, task['id'])}


def get(conn, profile_id, task_id):
    task = _task(conn, profile_id, task_id)
    coverage, _ = _population(conn, profile_id, task)
    receipts = [dict(id=r['id'], step=r['step'], created_at=r['created_at'], result=json.loads(r['result_json'])) for r in conn.execute(
        'SELECT * FROM gl_accounting_task_receipts WHERE profile_id=? AND task_id=? ORDER BY rowid', (profile_id, task_id))]
    cancelled = bool(conn.execute('SELECT 1 FROM gl_accounting_task_cancellations WHERE profile_id=? AND task_id=?', (profile_id, task_id)).fetchone())
    exceptions = [r for r in coverage if r['status'] == 'exception']
    period = ledger._period(conn, profile_id, task['period_id'])
    next_step = 'prepare' if any(r['status'] == 'ready' for r in coverage) else 'post' if any(r['status'] == 'draft' for r in coverage) else None if exceptions else 'close'
    if period['state'] == 'closed':
        next_step = 'tax_finalize' if task['spec']['tax_workpaper_id'] else 'export_close'
        if task['spec']['tax_workpaper_id'] and _current_final(conn, profile_id, task):
            next_step = 'export_tax'
    completed = False
    if period['state'] == 'closed' and not exceptions:
        identity = _current_close(conn, profile_id, task)
        final = _current_final(conn, profile_id, task) if task['spec']['tax_workpaper_id'] else None
        completed = any(r['step'] == ('export_tax' if final else 'export_close')
            and r['result'].get('final_id' if final else 'id') == (final['final_id'] if final else (identity or {}).get('id'))
            for r in receipts) and (not task['spec']['tax_workpaper_id'] or bool(final))
    return dict(id=task_id, period_id=task['period_id'], spec=task['spec'], scope_revision=task['scope_revision'], created_at=task['created_at'],
        state='cancelled' if cancelled else 'attention' if exceptions else 'completed' if completed else 'active', source_count=len(coverage),
        coverage=coverage, exceptions=exceptions, receipts=receipts, next_step=None if cancelled or completed else next_step)


def _current_close(conn, profile_id, task):
    return ledger._row(conn, '''SELECT e.id,e.period_id,e.revision,e.snapshot_digest FROM gl_period_events e
        JOIN gl_periods p ON p.profile_id=e.profile_id AND p.id=e.period_id
        WHERE e.profile_id=? AND e.period_id=? AND e.action='close' AND p.state='closed' AND e.revision=p.revision''',
        (profile_id, task['period_id']))


def _current_final(conn, profile_id, task):
    if not task['spec']['tax_workpaper_id']:
        return None
    from .tax_workpapers import preview_workpaper
    report = preview_workpaper(conn, profile_id, workpaper_id=task['spec']['tax_workpaper_id'])
    if not report['ready']:
        return None
    return ledger._row(conn, '''SELECT id AS final_id,report_digest,input_digest FROM gl_tax_finals
        WHERE profile_id=? AND workpaper_id=? AND input_digest=?''',
        (profile_id, task['spec']['tax_workpaper_id'], report['input_digest']))


def preview(conn, profile_id, task_id, step):
    if not isinstance(step, str) or step not in STEPS:
        _fail()
    task = _task(conn, profile_id, task_id)
    result = get(conn, profile_id, task_id)
    coverage, proposals = _population(conn, profile_id, task)
    blockers, detail = [], {}
    if result['state'] == 'cancelled':
        blockers.append({'kind': 'task_cancelled'})
    book = ledger.require_book(conn, profile_id)
    period = ledger._period(conn, profile_id, task['period_id'])
    if step == 'prepare':
        if period['state'] != 'open':
            blockers.append({'kind': 'period_not_open'})
        if not proposals:
            blockers.append({'kind': 'nothing_to_prepare'})
        if not blockers:
            conn.execute('SAVEPOINT accounting_task_prepare_preview')
            try:
                for proposal in proposals:
                    ledger.create_draft(conn, profile_id, proposal['payload'])
            except AppError as exc:
                blockers.append({'kind': exc.code})
            finally:
                conn.execute('ROLLBACK TO accounting_task_prepare_preview')
                conn.execute('RELEASE accounting_task_prepare_preview')
    elif step == 'post':
        ids = sorted({r['entry_id'] for r in coverage if r['status'] == 'draft'})
        if not ids:
            blockers.append({'kind': 'nothing_to_post'})
        detail['draft_ids'] = ids
        detail['entries'] = [ledger._entry(conn, profile_id, i) for i in ids]
        # Existing ledger validation may simulate writes but never retain them.
        for start in range(0, len(ids), 50):
            posting_batch.preview(conn, profile_id, draft_ids=ids[start:start + 50])
    elif step == 'close':
        detail = ledger.close_readiness(conn, profile_id, period_id=task['period_id'])
        detail['trial_balance'] = ledger.trial_balance(conn, profile_id, period_id=task['period_id'])
        detail['statements'] = ledger.financial_statements(conn, profile_id, period_id=task['period_id'])
        blockers.extend(detail['blockers'])
        if any(r['status'] not in ('posted', 'covered') for r in coverage):
            blockers.append({'kind': 'task_sources_unresolved'})
    elif step == 'tax_finalize':
        if not task['spec']['tax_workpaper_id']:
            blockers.append({'kind': 'tax_workpaper_not_selected'})
        else:
            from .tax_workpapers import preview_workpaper
            detail = preview_workpaper(conn, profile_id, workpaper_id=task['spec']['tax_workpaper_id'])
            blockers.extend(detail['blockers'])
    else:
        target = 'close' if step == 'export_close' else 'tax_finalize'
        artifact = _current_close(conn, profile_id, task) if step == 'export_close' else _current_final(conn, profile_id, task)
        if not artifact:
            blockers.append({'kind': 'task_' + target + '_required'})
        else:
            detail = artifact
    journal = ledger._row(conn, 'SELECT journal_input_version FROM profiles WHERE id=?', (profile_id,))
    assignments = ledger._rows(conn, 'SELECT id,request_digest FROM gl_accounting_task_evidence_assignments WHERE profile_id=? ORDER BY id', (profile_id,))
    binding = dict(profile_id=profile_id, task_id=task_id, step=step, revision=book['revision'], period=period,
        spec=task['spec'], coverage=coverage, proposals=proposals, detail=detail, blockers=blockers,
        rules=_rules(conn, profile_id), assignments=assignments, journal_input_version=journal['journal_input_version'])
    return {**result, 'step': step, 'expected_revision': book['revision'], 'expected_digest': ledger.digest(binding),
            'ready': not blockers, 'proposals': proposals if step == 'prepare' else [], 'blockers': blockers, 'detail': detail}


def _apply(conn, profile_id, p):
    required = {'task_id', 'step', 'expected_digest', 'expected_revision', 'idempotency_key', 'confirmed'}
    _fields(p, required | {'confirm_plaintext'}, required)
    if p['confirmed'] is not True or (p['step'] in ('export_close', 'export_tax') and p.get('confirm_plaintext') is not True):
        _fail('accounting_task_consent_required')
    ledger._text(p['idempotency_key'], 'idempotency_key', maximum=128)
    _task(conn, profile_id, p['task_id'])
    request_digest = ledger.digest(p)
    prior = ledger._row(conn, 'SELECT * FROM gl_accounting_task_receipts WHERE profile_id=? AND idempotency_key=?', (profile_id, p['idempotency_key']))
    if prior:
        if prior['request_digest'] != request_digest:
            _fail('accounting_idempotency_conflict')
        result = json.loads(prior['result_json'])
        output = result
        if prior['step'] == 'export_close':
            from .package import export_close
            output = {**export_close(conn, profile_id, close_id=result['id']), 'artifact_state': 'prepared'}
        elif prior['step'] == 'export_tax':
            from .tax_workpapers import export_workpaper
            output = {**export_workpaper(conn, profile_id, final_id=result['final_id'], confirm_plaintext=True), 'artifact_state': 'prepared'}
        return dict(task=get(conn, profile_id, p['task_id']), receipt={'id': prior['id'], 'step': prior['step'], 'result': result}, result=output, already_applied=True)
    reviewed = preview(conn, profile_id, p['task_id'], p['step'])
    if type(p['expected_revision']) is not int or reviewed['expected_revision'] != p['expected_revision'] or reviewed['expected_digest'] != p['expected_digest']:
        _fail('accounting_stale_approval')
    if not reviewed['ready']:
        _fail('accounting_task_blocked')
    step = p['step']
    if step == 'prepare':
        ids = []
        for proposal in reviewed['proposals']:
            entry = ledger.create_draft(conn, profile_id, proposal['payload'])
            conn.execute('INSERT INTO gl_accounting_task_claims VALUES(?,?,?,?,?)',
                (profile_id, proposal['source_kind'], proposal['source_id'], p['task_id'], entry['id']))
            ids.append(entry['id'])
        result = {'draft_ids': ids}
    elif step == 'post':
        ids = reviewed['detail']['draft_ids']
        for start in range(0, len(ids), 50):
            chunk = ids[start:start + 50]
            batch = posting_batch.preview(conn, profile_id, draft_ids=chunk)
            posting_batch.post(conn, profile_id, draft_ids=chunk, expected_revision=batch['expected_revision'], expected_digest=batch['expected_digest'],
                idempotency_key='task-' + ledger.digest([p['task_id'], p['idempotency_key'], start]), reason='Explicitly approved accounting task posting')
        for item in reviewed['coverage']:
            if item['source_kind'] != 'bank' or item.get('entry_id') not in ids:
                continue
            row = ledger._row(conn, 'SELECT r.*,s.account_code FROM gl_bank_rows r JOIN gl_bank_statements s ON s.id=r.statement_id WHERE r.profile_id=? AND r.id=?', (profile_id, item['source_id']))
            entry = ledger._entry(conn, profile_id, item['entry_id'])
            line = next(l for l in entry['lines'] if l['account_code'] == row['account_code'])
            bank.allocate_bank_row(conn, profile_id, row_id=row['id'], line_id=line['id'], amount_minor=abs(row['amount_minor']), idempotency_key='task-bank-' + row['id'])
        result = {'posted_ids': ids}
    elif step == 'close':
        result = ledger.close_period(conn, profile_id, period_id=reviewed['period_id'], expected_revision=p['expected_revision'])
    elif step == 'tax_finalize':
        from .tax_workpapers import finalize_workpaper
        detail = reviewed['detail']
        result = finalize_workpaper(conn, profile_id, workpaper_id=reviewed['spec']['tax_workpaper_id'],
            expected_revision=detail['binding']['revision'], expected_digest=detail['input_digest'])
    elif step == 'export_close':
        from .package import export_close
        result = export_close(conn, profile_id, close_id=reviewed['detail']['id'])
    else:
        from .tax_workpapers import export_workpaper
        result = export_workpaper(conn, profile_id, final_id=reviewed['detail']['final_id'], confirm_plaintext=True)
    # Export bytes stay in the explicit response; receipts retain identities only.
    retained = {key: value for key, value in result.items() if key in ('id', 'final_id', 'period_id', 'revision', 'snapshot_digest', 'report_digest', 'input_digest', 'draft_ids', 'posted_ids')}
    if step in ('export_close', 'export_tax'):
        retained['artifact_state'] = 'prepared'
        result['artifact_state'] = 'prepared'
    identifier = uuid4().hex
    conn.execute('INSERT INTO gl_accounting_task_receipts(id,profile_id,task_id,step,idempotency_key,request_digest,result_json) VALUES(?,?,?,?,?,?,?)',
        (identifier, profile_id, p['task_id'], step, p['idempotency_key'], request_digest, ledger.canonical_json(retained)))
    return dict(task=get(conn, profile_id, p['task_id']), receipt={'id': identifier, 'step': step, 'result': retained}, result=result, already_applied=False)


def execute(conn, profile_id, action, payload):
    ledger.require_book(conn, profile_id)
    if action not in READ_ACTIONS | WRITE_ACTIONS or not isinstance(payload, dict):
        _fail()
    with ledger.atomic(conn):
        if action == 'task-create':
            return _create(conn, profile_id, payload)
        if action == 'task-apply':
            return _apply(conn, profile_id, payload)
        if action == 'task-amend-preview':
            return _amend_preview(conn, profile_id, payload)
        if action == 'task-amend':
            return _amend(conn, profile_id, payload)
        if action == 'task-source-assign':
            return _assign_source(conn, profile_id, payload)
        if action == 'rule-create':
            return _create_rule(conn, profile_id, payload)
        if action in ('task-list', 'rule-list'):
            _fields(payload, set())
            if action == 'rule-list':
                return {'rules': _rules(conn, profile_id)}
            return {'tasks': [get(conn, profile_id, r[0]) for r in conn.execute(
                'SELECT id FROM gl_accounting_tasks WHERE profile_id=? ORDER BY created_at,id', (profile_id,))]}
        if action in ('task-cancel', 'rule-revoke'):
            key = 'task_id' if action == 'task-cancel' else 'rule_id'
            _fields(payload, {key, 'reason'}, {key, 'reason'})
            ledger._text(payload['reason'], 'reason', maximum=2000)
            if action == 'task-cancel':
                _task(conn, profile_id, payload[key])
                table = 'gl_accounting_task_cancellations'
            else:
                if not any(r['id'] == payload[key] for r in _rules(conn, profile_id)):
                    _fail('accounting_task_rule_not_found')
                table = 'gl_accounting_task_rule_revocations'
            prior = ledger._row(conn, f'SELECT * FROM {table} WHERE profile_id=? AND {key}=?', (profile_id, payload[key]))
            if prior and prior['reason'] != payload['reason']:
                _fail('accounting_idempotency_conflict')
            if not prior:
                conn.execute(f'INSERT INTO {table} VALUES(?,?,?)', (payload[key], profile_id, payload['reason']))
            return get(conn, profile_id, payload[key]) if action == 'task-cancel' else {key: payload[key], 'revoked': True}
        required = {'task_id', 'step'} if action == 'task-preview' else {'task_id'}
        _fields(payload, required, required)
        return preview(conn, profile_id, payload['task_id'], payload['step']) if action == 'task-preview' else get(conn, profile_id, payload['task_id'])
