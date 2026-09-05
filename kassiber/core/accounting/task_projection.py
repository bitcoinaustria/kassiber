"""Explicit Bitcoin task sources composed through reviewed projection services.

Only the chronological frontier is prepared: cumulative currency rounding and
transit settlement require the previous projection to be published first.
"""
from datetime import datetime
from uuid import uuid4

from ...errors import AppError
from . import artifacts, ledger, projection, projection_views, sources


def _fail(code='accounting_task_projection_scope'):
    raise AppError('Review the selected Bitcoin task source locally', code=code)


def _fields(value, allowed, required):
    if not isinstance(value, dict) or set(value) - allowed or required - set(value):
        _fail()


def _context(conn, profile_id, period_id, selection):
    artifact = artifacts.require_calculation_current(conn, profile_id, selection['artifact_id'])
    policy = projection.get_policy(conn, profile_id, selection['policy_id'])
    if policy['policy']['period_id'] != period_id:
        _fail()
    rows, cursor = {}, None
    while True:
        page = projection_views.list_events(conn, profile_id, artifact_id=artifact['id'], period_id=period_id, limit=500, cursor=cursor)
        rows.update((row['event_id'], row) for row in page['rows'])
        cursor = page['next_cursor']
        if cursor is None:
            break
    timestamps = {row['unique_id']: row['timestamp'] for row in artifact['capture']['inputs']['prepared_transactions']}
    for identifier, row in rows.items():
        row['timestamp'] = timestamps[identifier]
    return artifact, policy, rows


def normalize(conn, profile_id, period_id, value, maximum):
    _fields(value, {'artifact_id', 'policy_id', 'events'}, {'artifact_id', 'policy_id', 'events'})
    for key in ('artifact_id', 'policy_id'):
        ledger._text(value[key], key, maximum=128)
    if not isinstance(value['events'], list) or not 1 <= len(value['events']) <= maximum:
        _fail()
    identifiers, events = set(), []
    for event in value['events']:
        _fields(event, {'event_id', 'binding_id', 'category'}, {'event_id'})
        for key, item in event.items():
            ledger._text(item, key, maximum=1000 if key == 'event_id' else 128)
        if event.get('binding_id'):
            sources.get_binding(conn, profile_id, event['binding_id'])
        if event['event_id'] in identifiers:
            _fail()
        identifiers.add(event['event_id'])
        events.append(dict(event))
    _, _, rows = _context(conn, profile_id, period_id, value)
    if identifiers - rows.keys():
        _fail()
    return dict(artifact_id=value['artifact_id'], policy_id=value['policy_id'], events=events)


def fold_assignment(spec, retained):
    event = next(row for row in spec['projection']['events'] if row['event_id'] == retained['event_id'])
    event.update(binding_id=retained['binding_id'], category=retained['category'], assignment_revision=retained['scope_revision'])


def _choice(event, row):
    category = event.get('category') or (row['categories'][0] if len(row['categories']) == 1 else None)
    binding_id = event.get('binding_id') or (row['existing_binding_ids'][0] if len(row['existing_binding_ids']) == 1 else None)
    if category not in row['categories']:
        _fail('accounting_task_projection_category_required')
    if not binding_id:
        _fail('accounting_task_projection_binding_required')
    return binding_id, category


def _request(task, event, row):
    binding_id, category = _choice(event, row)
    selection = task['spec']['projection']
    request = dict(policy_id=selection['policy_id'], artifact_id=selection['artifact_id'],
        event_id=event['event_id'], binding_id=binding_id, category=category, period_id=task['period_id'])
    return {**request, 'idempotency_key': 'task-projection-' + ledger.digest([task['id'], request, event.get('assignment_revision', 0)])}


def _require_bank_claims_available(conn, profile_id, request):
    binding = sources.get_binding(conn, profile_id, request['binding_id'])
    snapshot = sources.get_snapshot(conn, profile_id, binding['snapshot_id'])
    claimed = {row['source_id'] for row in binding['claims']}
    for row in snapshot['snapshot']['sources']:
        if row['kind'] != 'bank' or row['source_id'] not in claimed:
            continue
        bank_row = row['facts']['bank_row_id']
        if conn.execute("SELECT 1 FROM gl_accounting_task_claims WHERE profile_id=? AND source_kind='bank' AND source_id=?", (profile_id, bank_row)).fetchone() or conn.execute('''SELECT 1 FROM gl_bank_allocations a WHERE profile_id=? AND row_id=?
            AND NOT EXISTS(SELECT 1 FROM gl_bank_allocation_voids v WHERE v.allocation_id=a.id)''', (profile_id, bank_row)).fetchone():
            _fail('accounting_task_bank_source_already_booked')


def _prepare(conn, profile_id, request):
    _require_bank_claims_available(conn, profile_id, request)
    return projection.create_proposal(conn, profile_id, **request)


def financial_view(proposal):
    # Throwaway proposal/draft IDs and their hashes are not review effects.
    payload = proposal['proposal']
    return {key: payload[key] for key in ('request', 'quantitative_posting', 'lines', 'policy_digest', 'valuation_release_digest')}


def _simulate(conn, operation):
    conn.execute('SAVEPOINT accounting_task_projection_preview')
    try:
        return operation()
    finally:
        conn.execute('ROLLBACK TO accounting_task_projection_preview')
        conn.execute('RELEASE accounting_task_projection_preview')


def population(conn, profile_id, task):
    selection = task['spec'].get('projection')
    if not selection:
        return [], []
    coverage = [dict(source_kind='bitcoin', source_id=event['event_id'], status='exception') for event in selection['events']]
    try:
        artifact, policy, rows = _context(conn, profile_id, task['period_id'], selection)
    except AppError as error:
        return [{**row, 'exception': error.code} for row in coverage], []
    selected = sorted(selection['events'], key=lambda event: (
        datetime.fromisoformat(rows[event['event_id']]['timestamp']),
        rows[event['event_id']]['categories'] == ['transfer_receipt'], event['event_id']))
    coverage, proposals, pending = [], [], False
    for event in selected:
        info = rows[event['event_id']]
        item = dict(source_kind='bitcoin', source_id=event['event_id'], status='exception',
            asset=info['asset'], quantity_msat=info['quantity_msat'], entry_date=info['entry_date'],
            artifact_digest=artifact['payload_digest'], policy_digest=policy['payload_digest'])
        try:
            request = _request(task, event, info)
            existing = ledger._row(conn, '''SELECT id FROM gl_projection_proposals WHERE profile_id=? AND event_id=?
                AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=gl_projection_proposals.id)''', (profile_id, event['event_id']))
            saved = projection.get_proposal(conn, profile_id, existing['id']) if existing else None
            if saved and any(saved['proposal']['request'].get(key) != value for key, value in request.items() if key != 'idempotency_key'):
                _fail('accounting_task_projection_conflict')
            if saved and saved['published']:
                item.update(status='posted', proposal_id=saved['id'])
                if saved['draft_id']:
                    item['entry_id'] = saved['draft_id']
            elif pending:
                _fail('accounting_task_projection_prior_pending')
            elif saved:
                _simulate(conn, lambda: projection.post_proposal(conn, profile_id, proposal_id=saved['id'], expected_digest=saved['payload_digest']))
                item.update(status='draft', proposal_id=saved['id'], projection=financial_view(saved), proposal_digest=saved['payload_digest'])
                if saved['draft_id']:
                    item['entry_id'] = saved['draft_id']
            else:
                prepared = _simulate(conn, lambda: _prepare(conn, profile_id, request))
                if prepared['voided']:
                    _fail('accounting_projection_voided')
                item.update(status='ready', projection=financial_view(prepared))
                proposals.append(dict(source_kind='bitcoin', source_id=event['event_id'], request=request, projection=item['projection']))
        except AppError as error:
            item.update(exception=error.code)
        if item['status'] != 'posted':
            pending = True
        coverage.append(item)
    return coverage, proposals


def bank_holds(conn, profile_id, task):
    """A reviewed shared source claim is not permission for a second posting."""
    selection = task['spec'].get('projection')
    if not selection:
        return set()
    artifact = artifacts.get_calculation(conn, profile_id, selection['artifact_id'])
    snapshot = sources.get_snapshot(conn, profile_id, artifact['source_snapshot_id'])
    bank_sources = {row['source_id']: row['facts']['bank_row_id'] for row in snapshot['snapshot']['sources'] if row['kind'] == 'bank'}
    holds = set()
    # Only explicit selected event bindings, plus the active bindings on its
    # retained source slices; there is no implicit source claiming here.
    mapping = artifact['capture']['inputs']['source_event_map']
    for event in selection['events']:
        binding_ids = {event['binding_id']} if event.get('binding_id') else set()
        source_id = mapping.get(event['event_id'], {}).get('source_id')
        binding_ids.update(row[0] for row in conn.execute('SELECT binding_id FROM gl_source_claims WHERE profile_id=? AND source_id=?', (profile_id, source_id)))
        for binding_id in binding_ids:
            binding = sources.get_binding(conn, profile_id, binding_id)
            if not binding['voided']:
                holds.update(bank_sources[claim['source_id']] for claim in binding['claims'] if claim['source_id'] in bank_sources)
    return holds


def assignment_preview(conn, profile_id, p):
    from . import tasks
    keys = {'task_id', 'event_id', 'binding_id', 'category', 'reason'}
    _fields(p, keys, keys)
    for key, value in p.items():
        ledger._text(value, key, maximum=2000 if key == 'reason' else 1000 if key == 'event_id' else 128)
    task = tasks._task(conn, profile_id, p['task_id'])
    if tasks.get(conn, profile_id, task['id'])['state'] == 'cancelled':
        _fail('accounting_task_cancelled')
    period = ledger._period(conn, profile_id, task['period_id'], open_required=True)
    selection = task['spec'].get('projection')
    event = next((event for event in selection['events'] if event['event_id'] == p['event_id']), None) if selection else None
    if event is None:
        _fail()
    if conn.execute('''SELECT 1 FROM gl_projection_proposals p WHERE profile_id=? AND event_id=?
        AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''', (profile_id, p['event_id'])).fetchone():
        _fail('accounting_task_projection_already_prepared')
    artifact, policy, rows = _context(conn, profile_id, task['period_id'], selection)
    updated = {**event, 'binding_id': p['binding_id'], 'category': p['category'], 'assignment_revision': task['scope_revision'] + 1}
    request = _request(task, updated, rows[p['event_id']])
    proposal = _simulate(conn, lambda: _prepare(conn, profile_id, request))
    book = ledger.require_book(conn, profile_id)
    binding = dict(task=task, period=period, book=book, artifact_digest=artifact['payload_digest'],
        policy_digest=policy['payload_digest'], request=p, projection=financial_view(proposal))
    return dict(task_id=task['id'], period_id=task['period_id'], scope_revision=task['scope_revision'],
        expected_revision=book['revision'], expected_digest=ledger.digest(binding), projection=financial_view(proposal))


def assign(conn, profile_id, p):
    from . import tasks
    keys = {'task_id', 'event_id', 'binding_id', 'category', 'reason'}
    required = keys | {'kind', 'expected_revision', 'expected_digest', 'confirmed', 'idempotency_key'}
    _fields(p, required, required)
    if p['kind'] != 'projection' or p['confirmed'] is not True:
        _fail('accounting_task_consent_required')
    ledger._text(p['idempotency_key'], 'idempotency_key', maximum=128)
    tasks._task(conn, profile_id, p['task_id'])
    checksum = ledger.digest(p)
    prior = ledger._row(conn, 'SELECT * FROM gl_accounting_task_receipts WHERE profile_id=? AND idempotency_key=?', (profile_id, p['idempotency_key']))
    if prior:
        if prior['request_digest'] != checksum or prior['step'] != 'projection_assignment':
            _fail('accounting_idempotency_conflict')
        return dict(task=tasks.get(conn, profile_id, p['task_id']), receipt_id=prior['id'], already_applied=True)
    reviewed = assignment_preview(conn, profile_id, {key: p[key] for key in keys})
    if type(p['expected_revision']) is not int or reviewed['expected_revision'] != p['expected_revision'] or reviewed['expected_digest'] != p['expected_digest']:
        _fail('accounting_stale_approval')
    retained = {**{key: p[key] for key in keys}, 'scope_revision': reviewed['scope_revision'] + 1,
                'expected_revision': p['expected_revision'], 'expected_digest': p['expected_digest']}
    identity = uuid4().hex
    conn.execute('INSERT INTO gl_accounting_task_receipts(id,profile_id,task_id,step,idempotency_key,request_digest,result_json) VALUES(?,?,?,?,?,?,?)',
        (identity, profile_id, p['task_id'], 'projection_assignment', p['idempotency_key'], checksum, ledger.canonical_json(retained)))
    ledger._bump(conn, profile_id)
    return dict(task=tasks.get(conn, profile_id, p['task_id']), receipt_id=identity, already_applied=False)
