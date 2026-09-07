"""Reviewed source-to-book proposals; RP2 remains the calculation authority.

The explicit policy adopts a retained RP2 carrying basis for the book. It is
not a jurisdiction's legal approval and never mutates the tax calculation.
Quantities remain exact even when every rounded currency leg is zero.
"""
from __future__ import annotations

from datetime import date, datetime, time, timedelta, timezone
from decimal import Decimal, ROUND_HALF_EVEN, localcontext
import json
from uuid import uuid4
from zoneinfo import ZoneInfo

from ...errors import AppError
from ...time_utils import now_iso
from . import ledger
from .artifacts import require_calculation_current
from .sources import get_binding


def ensure_schema(conn):
    statements = (
        '''CREATE TABLE IF NOT EXISTS gl_projection_policies(
            id TEXT PRIMARY KEY,profile_id TEXT NOT NULL REFERENCES gl_books(profile_id),
            payload_json TEXT NOT NULL,payload_digest TEXT NOT NULL,created_at TEXT NOT NULL,
            UNIQUE(profile_id,id),UNIQUE(profile_id,payload_digest))''',
        '''CREATE TABLE IF NOT EXISTS gl_projection_proposals(
            id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,policy_id TEXT NOT NULL,artifact_id TEXT NOT NULL,
            binding_id TEXT NOT NULL,event_id TEXT NOT NULL,period_id TEXT NOT NULL,entry_date TEXT NOT NULL,
            draft_id TEXT,payload_json TEXT NOT NULL,payload_digest TEXT NOT NULL,idempotency_key TEXT NOT NULL,
            created_at TEXT NOT NULL,UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),UNIQUE(draft_id),
            FOREIGN KEY(profile_id,policy_id) REFERENCES gl_projection_policies(profile_id,id),
            FOREIGN KEY(profile_id,binding_id) REFERENCES gl_source_bindings(profile_id,id),
            FOREIGN KEY(profile_id,artifact_id) REFERENCES gl_calculation_artifacts(profile_id,id),
            FOREIGN KEY(profile_id,period_id) REFERENCES gl_periods(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_projection_publications(
            proposal_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,entry_id TEXT,created_at TEXT NOT NULL,
            FOREIGN KEY(profile_id,proposal_id) REFERENCES gl_projection_proposals(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_projection_voids(
            proposal_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,reason TEXT NOT NULL,reversal_id TEXT,
            entry_date TEXT NOT NULL,period_id TEXT NOT NULL,created_at TEXT NOT NULL,
            FOREIGN KEY(profile_id,proposal_id) REFERENCES gl_projection_proposals(profile_id,id))''',
    )
    for statement in statements:
        conn.execute(statement)
    for table, collision in (
        ('gl_projection_policies','id=NEW.id OR (profile_id=NEW.profile_id AND payload_digest=NEW.payload_digest)'),
        ('gl_projection_proposals','id=NEW.id OR draft_id=NEW.draft_id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)'),
        ('gl_projection_publications','proposal_id=NEW.proposal_id'),
        ('gl_projection_voids','proposal_id=NEW.proposal_id')):
        for action in ('UPDATE','DELETE'):
            conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()} BEFORE {action} ON {table}
                BEGIN SELECT RAISE(ABORT,'accounting_projection_retained'); END''')
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS(SELECT 1 FROM {table} WHERE {collision})
            BEGIN SELECT RAISE(ABORT,'accounting_projection_retained'); END''')
    conn.execute('''CREATE TRIGGER IF NOT EXISTS gl_projection_event_once BEFORE INSERT ON gl_projection_proposals
        WHEN EXISTS(SELECT 1 FROM gl_projection_proposals p WHERE p.profile_id=NEW.profile_id AND p.event_id=NEW.event_id
            AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id))
        BEGIN SELECT RAISE(ABORT,'accounting_projection_duplicate'); END''')


def _account(conn, profile_id, code, kinds):
    row = ledger._row(conn, 'SELECT * FROM gl_accounts WHERE profile_id=? AND code=?', (profile_id, code))
    if not row or row['kind'] not in kinds:
        raise AppError('Projection account is missing or has the wrong type', code='accounting_projection_account')
    return code


def configure_policy(conn, profile_id, *, asset_accounts, settlement_account, income_account,
                     capital_account, gain_account, fee_account, acknowledge_tax_book_basis, reason, period_id,
                     transit_accounts=None):
    ledger.require_book(conn, profile_id)
    ledger._period(conn, profile_id, period_id, open_required=True)
    if acknowledge_tax_book_basis is not True:
        raise AppError('Explicit review is required before adopting tax carrying basis in the book', code='accounting_projection_policy')
    if not isinstance(asset_accounts, dict) or not asset_accounts or set(asset_accounts) - {'BTC','LBTC'}:
        raise AppError('Configure explicit Bitcoin asset accounts', code='accounting_projection_policy')
    transit_accounts = {} if transit_accounts is None else transit_accounts
    if not isinstance(transit_accounts, dict) or set(transit_accounts) - set(asset_accounts):
        raise AppError('Transit accounts must identify configured assets', code='accounting_projection_policy')
    reason = ledger._text(reason, 'reason', maximum=2000)
    payload = dict(period_id=period_id, basis_policy='reviewed_rp2_carrying_basis', currency_rounding='half_even_cumulative_account_remainder_to_gain',
        fee_policy='ordinary_fees_expensed_transfer_basis_from_rp2', legal_approval=False, reason=reason,
        asset_accounts={asset: _account(conn, profile_id, code, ('asset',)) for asset, code in asset_accounts.items()},
        transit_accounts={asset: _account(conn, profile_id, code, ('asset',)) for asset, code in transit_accounts.items()},
        settlement_account=_account(conn, profile_id, settlement_account, ('asset','liability')),
        income_account=_account(conn, profile_id, income_account, ('income',)),
        capital_account=_account(conn, profile_id, capital_account, ('equity',)),
        gain_account=_account(conn, profile_id, gain_account, ('income',)),
        fee_account=_account(conn, profile_id, fee_account, ('expense',)))
    account_codes = list(payload['asset_accounts'].values()) + list(payload['transit_accounts'].values()) + [payload[key] for key in
        ('settlement_account','income_account','capital_account','gain_account','fee_account')]
    if len(set(account_codes)) != len(account_codes):
        raise AppError('Projection roles require distinct accounts', code='accounting_projection_account')
    checksum = ledger.digest(payload)
    with ledger.atomic(conn):
        found = ledger._row(conn, 'SELECT id FROM gl_projection_policies WHERE profile_id=? AND payload_digest=?', (profile_id, checksum))
        if found:
            return get_policy(conn, profile_id, found['id'])
        identity = uuid4().hex
        conn.execute('INSERT INTO gl_projection_policies VALUES(?,?,?,?,?)', (identity, profile_id, ledger.canonical_json(payload), checksum, now_iso()))
        ledger._bump(conn, profile_id)
        return get_policy(conn, profile_id, identity)


def get_policy(conn, profile_id, policy_id):
    ledger.require_book(conn, profile_id)
    row = ledger._row(conn, 'SELECT * FROM gl_projection_policies WHERE profile_id=? AND id=?', (profile_id, policy_id))
    if not row:
        raise AppError('Projection policy was not found in this book', code='not_found')
    payload = json.loads(row.pop('payload_json'))
    if ledger.digest(payload) != row['payload_digest']:
        raise AppError('Projection policy content changed', code='accounting_projection_corrupt')
    return {**row, 'policy': payload}


def _minor(value, exponent):
    result = int((Decimal(value) * Decimal(10)**exponent).quantize(Decimal(1), rounding=ROUND_HALF_EVEN))
    ledger.strict_minor(abs(result))
    return result


def _amount_msat(transaction, field):
    value = Decimal(transaction[field]) * Decimal(100_000_000_000)
    if value != value.to_integral_value():
        raise AppError('Projection quantity is not an exact atomic amount', code='accounting_calculation_quantity')
    return int(value)


def _event(capture, event_id, policy, category, exponent):
    if category == 'custody_move':
        from ._projection_events import same_asset_event
        return same_asset_event(capture,event_id,policy,exponent)
    if category in ('transfer_dispatch','transfer_receipt'):
        from ._projection_events import transfer_event
        return transfer_event(capture,event_id,policy,category,exponent)
    prepared = capture['inputs']['prepared_transactions']
    event = next((item for item in prepared if item['unique_id'] == event_id), None)
    if not event or event.get('execution_role') or event['kind'] not in ('IN','OUT'):
        raise AppError('Select an executed acquisition or disposal', code='accounting_projection_event')
    if any(item.get('out_id') == event_id or item.get('in_id') == event_id for item in capture['inputs'].get('custody_relations', [])):
        raise AppError('Custody relations require a linked transfer proposal', code='accounting_projection_event')
    asset_account = policy['asset_accounts'].get(event['asset'])
    if not asset_account:
        raise AppError('Asset account is not configured', code='accounting_projection_account')
    movements, exact = {}, {}
    def add(code, signed):
        movements[code] = movements.get(code, 0) + signed
    if event['kind'] == 'IN':
        if category not in ('purchase','income','capital'):
            raise AppError('Review the acquisition classification', code='accounting_projection_category')
        quantity = _amount_msat(event, 'crypto_in_exact')
        basis = Decimal(event['fiat_in_with_fee_exact'])
        amount = _minor(basis, exponent)
        add(asset_account, amount)
        add(policy[{'purchase':'settlement_account','income':'income_account','capital':'capital_account'}[category]], -amount)
        exact = dict(basis_exact=format(basis,'f'), quantity_msat=quantity, asset=event['asset'])
    else:
        if category != 'disposal':
            raise AppError('Disposals require explicit disposal classification', code='accounting_projection_category')
        principal = _amount_msat(event, 'crypto_out_no_fee_exact')
        fee = _amount_msat(event, 'crypto_fee_exact')
        execution = [item for item in capture['inputs'].get('execution_basis', []) if item['asset'] == event['asset'] and item['event_id'] == event_id]
        if sum(item['quantity_msat'] for item in execution) != principal + fee:
            raise AppError('Complete execution basis is required', code='accounting_projection_basis')
        left, principal_basis, total_basis = principal, Decimal(0), Decimal(0)
        for fragment in execution:
            total_basis += Decimal(fragment['basis_exact'])
            portion = min(left, fragment['quantity_msat'])
            principal_basis += Decimal(portion) / Decimal(100_000_000_000) * Decimal(fragment['unit_basis_exact'])
            left -= portion
        total_minor = _minor(total_basis, exponent)
        principal_minor = _minor(principal_basis, exponent)
        proceeds = _minor(event['fiat_out_no_fee_exact'], exponent)
        add(asset_account, -total_minor)
        add(policy['settlement_account'], proceeds)
        add(policy['fee_account'], total_minor - principal_minor)
        add(policy['gain_account'], principal_minor - proceeds)
        exact = dict(basis_exact=format(-total_basis,'f'), quantity_msat=-(principal+fee), asset=event['asset'],
                     principal_basis_exact=format(principal_basis,'f'), proceeds_exact=event['fiat_out_no_fee_exact'])
    lines = [dict(account_code=code, debit_minor=max(amount,0), credit_minor=max(-amount,0))
             for code, amount in sorted(movements.items()) if amount]
    exact.update(account_code=asset_account, location='inventory', book_value_minor=movements.get(asset_account,0))
    return event, exact, lines


def _require_binding(conn, profile_id, binding_id, capture, event_id, quantity, *, required_role='recognition'):
    binding = get_binding(conn, profile_id, binding_id)
    if binding['voided'] or binding['snapshot_id'] != capture['source_snapshot_id'] or binding['role'] != required_role:
        raise AppError('Projection requires a current reviewed source binding', code='accounting_projection_binding')
    mapping = capture['inputs']['source_event_map'].get(event_id, {})
    claims = [item for item in binding['claims'] if item['source_id'] == mapping.get('source_id') and item['unit'] == 'msat']
    def merged(rows):
        result = []
        for item in sorted(rows, key=lambda row: row['start_atomic']):
            start,end = item['start_atomic'],item['end_atomic']
            if result and result[-1][1] == start:
                result[-1] = (result[-1][0],end)
            else:
                result.append((start,end))
        return result
    expected = mapping.get('claim_slices', [])
    if not claims or merged(claims) != merged(expected) or sum(item['end_atomic']-item['start_atomic'] for item in expected) != abs(quantity):
        raise AppError('Reviewed source claim must exactly cover this projected event', code='accounting_projection_binding')
    return binding


def create_proposal(conn, profile_id, *, policy_id, artifact_id, binding_id, event_id, category, period_id, idempotency_key):
    with ledger.atomic(conn), localcontext() as context:
        context.prec = 32
        book = ledger.require_book(conn, profile_id)
        request = dict(policy_id=policy_id, artifact_id=artifact_id, binding_id=binding_id, event_id=event_id,
                       category=category, period_id=period_id, idempotency_key=idempotency_key)
        idempotency_key = ledger._text(idempotency_key, 'idempotency_key', maximum=200)
        existing = ledger._row(conn, 'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND idempotency_key=?', (profile_id,idempotency_key))
        if existing:
            saved = get_proposal(conn, profile_id, existing['id'])
            if saved['proposal']['request'] != request:
                raise AppError('Projection retry differs from the original request', code='accounting_idempotency_conflict')
            return saved
        capture = require_calculation_current(conn, profile_id, artifact_id)['capture']
        policy = get_policy(conn, profile_id, policy_id)['policy']
        if policy['period_id'] != period_id:
            raise AppError('Review the book basis policy for the selected period', code='accounting_projection_policy')
        event, exact, lines = _event(capture, event_id, policy, category, book['minor_unit_exponent'])
        from .valuation_releases import apply_to_event,retain_plan
        exact,lines,release_plan=apply_to_event(conn,profile_id,capture,event_id,event,exact,lines,policy,category)
        from .currency_rounding import apply_to_event as round_currency
        exact,lines=round_currency(conn,profile_id,event,exact,lines,policy)
        for claim in exact.get('required_claims', [dict(event_id=event_id,quantity_msat=exact['quantity_msat'])]):
            _require_binding(conn, profile_id, binding_id, capture, claim['event_id'], claim['quantity_msat'],
                required_role='settlement' if category in ('transfer_dispatch','transfer_receipt','custody_move') else 'recognition')
        if conn.execute('''SELECT 1 FROM gl_projection_proposals p WHERE profile_id=? AND event_id=?
            AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''', (profile_id,event_id)).fetchone():
            raise AppError('This source event already has a projection', code='accounting_projection_duplicate')
        period = ledger._period(conn, profile_id, period_id, open_required=True)
        day = datetime.fromisoformat(event['timestamp']).astimezone(ZoneInfo(book['timezone'])).date().isoformat()
        if not period['start_date'] <= day <= period['end_date']:
            raise AppError('Projected source event is outside the selected period', code='accounting_projection_period')
        ledger._check_opening_cutoff(conn, profile_id, day)
        identity = uuid4().hex
        draft = ledger.create_draft(conn, profile_id, dict(period_id=period_id, entry_date=day,
            description=f'Reviewed {category}: {event_id}', source_ref=f'projection:{identity}',
            idempotency_key=f'projection:{identity}', lines=lines)) if lines else None
        payload = dict(request=request, quantitative_posting=exact, lines=lines,
                       draft_digest=draft['payload_digest'] if draft else None, policy_digest=ledger.digest(policy),
                       valuation_release_digest=ledger.digest(release_plan) if release_plan is not None else None)
        checksum = ledger.digest(payload)
        conn.execute('INSERT INTO gl_projection_proposals VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)',
            (identity,profile_id,policy_id,artifact_id,binding_id,event_id,period_id,day,
             draft['id'] if draft else None,ledger.canonical_json(payload),checksum,idempotency_key,now_iso()))
        retain_plan(conn,profile_id,identity,release_plan)
        ledger._bump(conn, profile_id)
        return get_proposal(conn, profile_id, identity)


def get_proposal(conn, profile_id, proposal_id):
    ledger.require_book(conn, profile_id)
    row = ledger._row(conn, 'SELECT * FROM gl_projection_proposals WHERE profile_id=? AND id=?', (profile_id, proposal_id))
    if not row:
        raise AppError('Projection proposal was not found in this book', code='not_found')
    payload = json.loads(row.pop('payload_json'))
    if ledger.digest(payload) != row['payload_digest']:
        raise AppError('Projection content changed', code='accounting_projection_corrupt')
    row['published'] = bool(conn.execute('SELECT 1 FROM gl_projection_publications WHERE profile_id=? AND proposal_id=?', (profile_id,proposal_id)).fetchone())
    row['voided'] = bool(conn.execute('SELECT 1 FROM gl_projection_voids WHERE profile_id=? AND proposal_id=?', (profile_id,proposal_id)).fetchone())
    return {**row, 'proposal': payload}


def _validate_proposal(conn, profile_id, proposal):
    if proposal['voided']:
        raise AppError('Projection was discarded or reversed', code='accounting_projection_voided')
    capture = require_calculation_current(conn, profile_id, proposal['artifact_id'])['capture']
    if proposal['proposal']['request']['category']=='opening':
        from .opening import validate_binding
        validate_binding(conn,profile_id,proposal)
        if conn.execute("SELECT 1 FROM gl_entries WHERE profile_id=? AND status='posted'",(profile_id,)).fetchone() or conn.execute(
            'SELECT 1 FROM gl_projection_publications WHERE profile_id=?',(profile_id,)).fetchone():
            raise AppError('Opening migration must precede posted book activity',code='accounting_projection_period')
        return
    exact = proposal['proposal']['quantitative_posting']
    for claim in exact.get('required_claims',[dict(event_id=proposal['event_id'],quantity_msat=exact['quantity_msat'])]):
        _require_binding(conn, profile_id, proposal['binding_id'], capture, claim['event_id'], claim['quantity_msat'],
            required_role='settlement' if proposal['proposal']['request']['category'] in ('transfer_dispatch','transfer_receipt','custody_move') else 'recognition')
    from .valuation_releases import validate_proposal
    validate_proposal(conn,profile_id,proposal,capture)
    if proposal['proposal']['request']['category'] == 'transfer_receipt':
        relation = next(item for item in capture['inputs']['custody_relations'] if item.get('in_id') == proposal['event_id'])
        from ._projection_events import published_transit_origin
        origin=published_transit_origin(conn,profile_id,relation)
        if not origin:
            raise AppError('Post the reviewed transfer dispatch before its receipt', code='accounting_projection_dispatch_required')
        _,transit=origin
        if not transit or transit['account_code'] != exact['account_code'] or transit['quantity_msat'] != -exact['quantity_msat'] or Decimal(transit['basis_exact']) != -Decimal(exact['basis_exact']):
            raise AppError('Receipt must exactly settle its published transit position', code='accounting_projection_transit_mismatch')
    ledger._period(conn, profile_id, proposal['period_id'], open_required=True)


def validate_draft(conn, profile_id, entry):
    row = ledger._row(conn, 'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND draft_id=?', (profile_id,entry['id']))
    if row:
        proposal = get_proposal(conn, profile_id, row['id'])
        _validate_proposal(conn, profile_id, proposal)
        if proposal['proposal']['draft_digest'] != entry['payload_digest']:
            raise AppError('Projection draft approval changed', code='accounting_stale_approval')


def after_post(conn, profile_id, entry_id):
    row = ledger._row(conn, 'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND draft_id=?', (profile_id,entry_id))
    if row and not conn.execute('SELECT 1 FROM gl_projection_publications WHERE proposal_id=?', (row['id'],)).fetchone():
        conn.execute('INSERT INTO gl_projection_publications VALUES(?,?,?,?)', (row['id'],profile_id,entry_id,now_iso()))


def post_proposal(conn, profile_id, *, proposal_id, expected_digest):
    with ledger.atomic(conn):
        proposal = get_proposal(conn, profile_id, proposal_id)
        if proposal['payload_digest'] != expected_digest:
            raise AppError('Projection approval changed', code='accounting_stale_approval')
        if proposal['published']:
            return proposal
        _validate_proposal(conn, profile_id, proposal)
        if proposal['draft_id']:
            ledger.post_draft(conn, profile_id, draft_id=proposal['draft_id'], expected_digest=proposal['proposal']['draft_digest'])
            after_post(conn, profile_id, proposal['draft_id'])
        else:
            conn.execute('INSERT INTO gl_projection_publications VALUES(?,?,NULL,?)', (proposal_id,profile_id,now_iso()))
        ledger._bump(conn, profile_id)
        return get_proposal(conn, profile_id, proposal_id)


def before_discard(conn, profile_id, entry):
    row = ledger._row(conn, 'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND draft_id=?', (profile_id,entry['id']))
    if row:
        conn.execute('INSERT INTO gl_projection_voids VALUES(?,?,?,?,?,?,?)',
            (row['id'],profile_id,'Draft discarded',None,entry['entry_date'],entry['period_id'],now_iso()))


def after_reverse(conn, profile_id, original_id, reversal_id):
    row = ledger._row(conn, 'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND draft_id=?', (profile_id,original_id))
    if row:
        reversal = ledger._entry(conn, profile_id, reversal_id)
        conn.execute('INSERT INTO gl_projection_voids VALUES(?,?,?,?,?,?,?)',
            (row['id'],profile_id,'Posted entry reversed',reversal_id,reversal['entry_date'],reversal['period_id'],now_iso()))


def void_quantity_proposal(conn, profile_id, *, proposal_id, expected_digest, entry_date, period_id, reason):
    """Append a dated reversal for a zero-fiat publication, or discard its draft."""
    with ledger.atomic(conn):
        proposal = get_proposal(conn, profile_id, proposal_id)
        reason = ledger._text(reason, 'reason', maximum=2000)
        if proposal['draft_id'] or expected_digest != proposal['payload_digest']:
            raise AppError('Use the linked journal entry correction workflow', code='accounting_stale_approval')
        if proposal['voided']:
            previous = ledger._row(conn, 'SELECT * FROM gl_projection_voids WHERE proposal_id=?', (proposal_id,))
            if (previous['entry_date'],previous['period_id'],previous['reason']) != (entry_date,period_id,reason):
                raise AppError('Quantity reversal differs from its first approval', code='accounting_idempotency_conflict')
            return proposal
        _require_proposal_reversible(conn,profile_id,proposal)
        period = ledger._period(conn, profile_id, period_id, open_required=True)
        if not period['start_date'] <= entry_date <= period['end_date'] or entry_date < proposal['entry_date']:
            raise AppError('Quantity reversal needs an open period and cannot precede its original', code='accounting_projection_period')
        conn.execute('INSERT INTO gl_projection_voids VALUES(?,?,?,?,?,?,?)',
            (proposal_id,profile_id,reason,None,entry_date,period_id,now_iso()))
        ledger._bump(conn, profile_id)
        return get_proposal(conn, profile_id, proposal_id)


def require_binding_releasable(conn, profile_id, binding_id):
    if conn.execute('''SELECT 1 FROM gl_projection_proposals p WHERE profile_id=? AND binding_id=?
        AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''', (profile_id,binding_id)).fetchone():
        raise AppError('Discard or reverse the linked projection before releasing its sources', code='accounting_projection_in_use')


def _require_proposal_reversible(conn, profile_id, proposal):
    from .valuation_releases import require_proposal_reversible
    require_proposal_reversible(conn,profile_id,proposal['id'])
    if proposal['proposal']['request']['category'] != 'transfer_dispatch':
        return
    relation_id = proposal['proposal']['quantitative_posting']['relation_id']
    for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p JOIN gl_projection_publications x ON x.proposal_id=p.id
        WHERE p.profile_id=? AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''',(profile_id,)):
        other = get_proposal(conn,profile_id,row[0])
        if other['proposal']['request']['category']=='transfer_receipt' and other['proposal']['quantitative_posting'].get('relation_id')==relation_id:
            raise AppError('Reverse the linked transfer receipt before its dispatch', code='accounting_projection_in_use')


def require_reversible(conn, profile_id, entry_id):
    row = ledger._row(conn,'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND draft_id=?',(profile_id,entry_id))
    if row:
        _require_proposal_reversible(conn,profile_id,get_proposal(conn,profile_id,row['id']))


def quantity_rows(proposal):
    main = proposal['proposal']['quantitative_posting']
    return [{key:main[key] for key in ('asset','quantity_msat','basis_exact','account_code','location','book_value_minor','relation_id') if key in main},
            *main.get('related_postings',[])]


def validate_close(conn, profile_id, start_date, end_date):
    """Reconcile captured custody/basis with cumulative published book inputs.

    Returns explicit controls for the root's close snapshot. It never asserts
    that the user imported every external transaction, invoice or wallet.
    """
    from .artifacts import get_calculation
    from .sources import preview_sources, source_coverage

    book = ledger.require_book(conn,profile_id)
    current = preview_sources(conn,profile_id)
    relevant = [item for item in current['sources'] if item['kind']=='custody' and item['occurred_on']<=end_date]
    retained_activity=conn.execute('''SELECT 1 FROM gl_projection_proposals p
        WHERE p.profile_id=? AND p.entry_date<=? AND NOT EXISTS(
            SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id AND v.entry_date<=?)''',
        (profile_id,end_date,end_date)).fetchone()
    if not relevant and not retained_activity:
        return dict(required=False,blockers=[],external_completeness_verified=False)
    cutoff = datetime.combine(date.fromisoformat(end_date)+timedelta(days=1),time.min,
                              ZoneInfo(book['timezone'])).astimezone(timezone.utc).isoformat().replace('+00:00','Z')
    artifact = None
    for row in conn.execute('SELECT id FROM gl_calculation_artifacts WHERE profile_id=? ORDER BY created_at DESC,id DESC',(profile_id,)):
        candidate = get_calculation(conn,profile_id,row[0])
        if candidate['capture']['cutoff_exclusive_utc']==cutoff:
            # A valid older capture must not lose to a stale record sharing the
            # second-resolution created_at timestamp (UUID order is random).
            if artifact is None:
                artifact = candidate
            try:
                require_calculation_current(conn,profile_id,candidate['id'])
            except AppError:
                continue
            artifact = candidate
            break
    if artifact is None:
        return dict(required=True,blockers=[dict(code='accounting_close_calculation_required')],external_completeness_verified=False)
    blockers = []
    try:
        require_calculation_current(conn,profile_id,artifact['id'])
    except AppError as error:
        blockers.append(dict(code=error.code))
    from .valuation import controls as valuation_controls
    book_adjustments=valuation_controls(conn,profile_id,end_date)
    blockers.extend(book_adjustments['blockers'])
    expected,actual,book_values,account_basis,records = {},{},{},{},[]
    with localcontext() as context:
        context.prec=32
        for asset in artifact['capture']['assets']:
            expected[asset['asset']] = dict(quantity_msat=sum(item['quantity_msat'] for item in asset['open_positions']),
                basis=sum((Decimal(item['basis_exact']) for item in asset['open_positions']),Decimal(0)))
        for relation in artifact['capture']['inputs'].get('cutoff_relations',[]):
            if relation.get('future_reference') and relation.get('basis_carried_exact') is not None:
                target=expected.setdefault(relation['to_asset'],dict(quantity_msat=0,basis=Decimal(0)))
                target['quantity_msat']+=relation['quantity_received_msat']
                target['basis']+=Decimal(relation['basis_carried_exact'])
        for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p JOIN gl_projection_publications x ON x.proposal_id=p.id
            WHERE p.profile_id=? AND p.entry_date<=? ORDER BY p.entry_date,p.id''',(profile_id,end_date)):
            proposal=get_proposal(conn,profile_id,row[0])
            void=ledger._row(conn,'SELECT * FROM gl_projection_voids WHERE profile_id=? AND proposal_id=? AND entry_date<=?',
                             (profile_id,proposal['id'],end_date))
            records.append(dict(proposal=proposal,void=void))
            if void:
                continue
            for item in quantity_rows(proposal):
                target=actual.setdefault(item['asset'],dict(quantity_msat=0,basis=Decimal(0)))
                target['quantity_msat']+=item['quantity_msat']
                target['basis']+=Decimal(item['basis_exact'])
                book_values[item['account_code']]=book_values.get(item['account_code'],0)+item['book_value_minor']
                account_basis[item['account_code']]=account_basis.get(item['account_code'],Decimal(0))+Decimal(item['basis_exact'])
        matrix=[]
        scale=Decimal(10)**book['minor_unit_exponent']
        for asset,adjustment in book_adjustments['remaining_by_asset_minor'].items():
            target=expected.setdefault(asset,dict(quantity_msat=0,basis=Decimal(0)))
            target['basis']+=Decimal(adjustment)/scale
        for asset,adjustment in book_adjustments['original_by_asset_minor'].items():
            target=actual.setdefault(asset,dict(quantity_msat=0,basis=Decimal(0)))
            target['basis']+=Decimal(adjustment)/scale
        for code,amount in book_adjustments['account_adjustments_minor'].items():
            book_values[code]=book_values.get(code,0)+amount
            account_basis[code]=account_basis.get(code,Decimal(0))+Decimal(amount)/scale
        for asset in sorted(set(expected)|set(actual)):
            wanted=expected.get(asset,dict(quantity_msat=0,basis=Decimal(0)))
            have=actual.get(asset,dict(quantity_msat=0,basis=Decimal(0)))
            quantity_difference=have['quantity_msat']-wanted['quantity_msat']
            basis_difference=have['basis']-wanted['basis']
            matrix.append(dict(asset=asset,expected_quantity_msat=wanted['quantity_msat'],book_quantity_msat=have['quantity_msat'],
                quantity_difference_msat=quantity_difference,expected_basis_exact=format(wanted['basis'],'f'),
                tax_basis_exact=format(wanted['basis']-Decimal(book_adjustments['remaining_by_asset_minor'].get(asset,0))/scale,'f'),
                book_adjustment_minor=book_adjustments['remaining_by_asset_minor'].get(asset,0),
                book_basis_exact=format(have['basis'],'f'),basis_difference_exact=format(basis_difference,'f')))
            if quantity_difference or basis_difference:
                blockers.append(dict(code='accounting_projection_reconciliation',asset=asset))
        for code,expected_minor in book_values.items():
            if expected_minor!=_minor(account_basis[code],book['minor_unit_exponent']):
                blockers.append(dict(code='accounting_projection_currency_remainder',account_code=code))
            balance=sum(row[0]-row[1] for row in conn.execute('''SELECT l.debit_minor,l.credit_minor FROM gl_lines l JOIN gl_entries e
                ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE l.profile_id=? AND l.account_code=? AND e.status='posted' AND e.entry_date<=?''',
                (profile_id,code,end_date)))
            if balance!=expected_minor:
                blockers.append(dict(code='accounting_projection_gl_difference',account_code=code))
    coverage=source_coverage(conn,profile_id)
    relevant_ids={item['source_id'] for item in relevant}
    from .sources import get_snapshot
    for stale in coverage['stale_bindings']:
        binding=get_binding(conn,profile_id,stale['binding_id'])
        saved=get_snapshot(conn,profile_id,binding['snapshot_id'])['snapshot']['sources']
        original=next((item for item in saved if item['source_id']==stale['source_id']),None)
        if stale['source_id'] in relevant_ids or (original and original['occurred_on']<=end_date):
            blockers.append(dict(code='accounting_projection_source_stale',**stale))
    for row in coverage['rows']:
        if row['source_id'] in relevant_ids and row['remaining_atomic']:
            blockers.append(dict(code='accounting_projection_source_unassigned',source_id=row['source_id']))
    drafts=[dict(row) for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p WHERE p.profile_id=? AND p.entry_date<=?
        AND NOT EXISTS(SELECT 1 FROM gl_projection_publications x WHERE x.proposal_id=p.id)
        AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''',(profile_id,end_date))]
    blockers.extend(dict(code='accounting_projection_unpublished',proposal_id=row['id']) for row in drafts)
    return dict(required=True,blockers=blockers,reconciliation=matrix,calculation_artifact=artifact,
                publications=records,source_coverage=coverage,book_adjustments=book_adjustments,external_completeness_verified=False)
