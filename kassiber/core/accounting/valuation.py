"""Evidence-backed book-only valuation layers over authoritative RP2 lots.

No lot is selected here. Releases follow the exact lot quantities selected by
RP2's retained execution capture. Currency allocation is deterministic integer
arithmetic; tax acquisitions, pools, results and overrides remain unchanged.
"""
from datetime import date, datetime, time, timedelta, timezone
from decimal import Decimal
import json
from uuid import uuid4
from zoneinfo import ZoneInfo

from ...errors import AppError
from ...time_utils import now_iso
from . import ledger
from .artifacts import require_calculation_current


def ensure_schema(conn):
    for statement in (
        '''CREATE TABLE IF NOT EXISTS gl_book_valuations(
            id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,policy_id TEXT NOT NULL,artifact_id TEXT NOT NULL,
            period_id TEXT NOT NULL,effective_date TEXT NOT NULL,asset TEXT NOT NULL,account_code TEXT NOT NULL,
            evidence_id TEXT NOT NULL,draft_id TEXT NOT NULL UNIQUE,payload_json TEXT NOT NULL,payload_digest TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,created_at TEXT NOT NULL,UNIQUE(profile_id,id),UNIQUE(profile_id,idempotency_key),
            FOREIGN KEY(profile_id,policy_id) REFERENCES gl_projection_policies(profile_id,id),
            FOREIGN KEY(profile_id,artifact_id) REFERENCES gl_calculation_artifacts(profile_id,id),
            FOREIGN KEY(profile_id,period_id) REFERENCES gl_periods(profile_id,id),
            FOREIGN KEY(profile_id,evidence_id) REFERENCES gl_evidence(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_valuation_publications(
            valuation_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,entry_id TEXT NOT NULL,created_at TEXT NOT NULL,
            FOREIGN KEY(profile_id,valuation_id) REFERENCES gl_book_valuations(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_valuation_voids(
            valuation_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,reversal_id TEXT,effective_date TEXT NOT NULL,
            reason TEXT NOT NULL,created_at TEXT NOT NULL,
            FOREIGN KEY(profile_id,valuation_id) REFERENCES gl_book_valuations(profile_id,id))''',
        '''CREATE TABLE IF NOT EXISTS gl_valuation_releases(
            proposal_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,payload_json TEXT NOT NULL,payload_digest TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY(profile_id,proposal_id) REFERENCES gl_projection_proposals(profile_id,id))''',
    ):
        conn.execute(statement)
    for table,collision in (
        ('gl_book_valuations','id=NEW.id OR draft_id=NEW.draft_id OR (profile_id=NEW.profile_id AND idempotency_key=NEW.idempotency_key)'),
        ('gl_valuation_publications','valuation_id=NEW.valuation_id'),('gl_valuation_voids','valuation_id=NEW.valuation_id'),
        ('gl_valuation_releases','proposal_id=NEW.proposal_id')):
        for action in ('UPDATE','DELETE'):
            conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {table}_no_{action.lower()} BEFORE {action} ON {table}
                BEGIN SELECT RAISE(ABORT,'accounting_valuation_retained'); END''')
        conn.execute(f'''CREATE TRIGGER IF NOT EXISTS {table}_no_replace BEFORE INSERT ON {table}
            WHEN EXISTS(SELECT 1 FROM {table} WHERE {collision}) BEGIN SELECT RAISE(ABORT,'accounting_valuation_retained'); END''')


def _enabled(conn):
    return bool(conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_book_valuations'").fetchone())


def _allocate(amount,weights):
    total=sum(weights.values())
    if not total:
        raise AppError('A valuation requires positive retained quantity',code='accounting_valuation_scope')
    sign=-1 if amount<0 else 1
    result={key:abs(amount)*weight//total for key,weight in weights.items()}
    remaining=abs(amount)-sum(result.values())
    order=sorted(weights,key=lambda key:(-(abs(amount)*weights[key]%total),key))
    for key in order[:remaining]:
        result[key]+=1
    return {key:sign*value for key,value in result.items()}


def get_valuation(conn,profile_id,valuation_id):
    ledger.require_book(conn,profile_id)
    valuation_id=ledger._text(valuation_id,'valuation_id',maximum=128)
    row=ledger._row(conn,'SELECT * FROM gl_book_valuations WHERE profile_id=? AND id=?',(profile_id,valuation_id))
    if not row:
        raise AppError('Book valuation was not found in this profile',code='not_found')
    payload=json.loads(row.pop('payload_json'))
    if ledger.digest(payload)!=row['payload_digest']:
        raise AppError('Book valuation commitment changed',code='accounting_valuation_corrupt')
    return {**row,'valuation':payload,
        'published':bool(conn.execute('SELECT 1 FROM gl_valuation_publications WHERE valuation_id=?',(valuation_id,)).fetchone()),
        'voided':bool(conn.execute('SELECT 1 FROM gl_valuation_voids WHERE valuation_id=?',(valuation_id,)).fetchone())}


def _published_proposals(conn,profile_id,end_date):
    from .projection import get_proposal
    return [get_proposal(conn,profile_id,row[0]) for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p
        JOIN gl_projection_publications x ON x.proposal_id=p.id WHERE p.profile_id=? AND p.entry_date<=?
        AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id AND v.entry_date<=?)
        ORDER BY x.rowid''',(profile_id,end_date,end_date))]


def _state(conn,profile_id,end_date):
    """Replay adjustment layers and their published quantity consumption."""
    if not _enabled(conn):
        return {},[],[]
    states,valuations,plans={},[],[]
    for row in conn.execute('''SELECT v.id FROM gl_book_valuations v JOIN gl_valuation_publications x ON x.valuation_id=v.id
        WHERE v.profile_id=? AND v.effective_date<=? AND NOT EXISTS(SELECT 1 FROM gl_valuation_voids z
            WHERE z.valuation_id=v.id AND z.effective_date<=?) ORDER BY v.effective_date,v.id''',(profile_id,end_date,end_date)):
        valuation=get_valuation(conn,profile_id,row[0])
        valuations.append(valuation)
        for lot in valuation['valuation']['scope_lots']:
            states[(valuation['id'],valuation['asset'],lot['lot_id'])]=dict(quantity_msat=lot['quantity_msat'],
                adjustment_minor=lot['adjustment_minor'],initial_quantity_msat=lot['quantity_msat'],
                initial_adjustment_minor=lot['adjustment_minor'],available_after=valuation['effective_date'],
                valuation_digest=valuation['payload_digest'])
    for proposal in _published_proposals(conn,profile_id,end_date):
        row=ledger._row(conn,'SELECT * FROM gl_valuation_releases WHERE profile_id=? AND proposal_id=?',(profile_id,proposal['id']))
        if not row:
            continue
        plan=json.loads(row['payload_json'])
        if ledger.digest(plan)!=row['payload_digest']:
            raise AppError('Book adjustment release commitment changed',code='accounting_valuation_corrupt')
        plans.append(dict(proposal_id=proposal['id'],plan=plan,digest=row['payload_digest']))
        for allocation in plan['allocations']:
            key=(allocation['valuation_id'],allocation['asset'],allocation['lot_id'])
            current=states.get(key)
            if current is None or current['valuation_digest']!=allocation['valuation_digest'] or current['quantity_msat']<allocation['quantity_msat'] or current['quantity_msat']!=allocation['before_quantity_msat'] or current['adjustment_minor']!=allocation['before_adjustment_minor']:
                raise AppError('Book adjustment release exceeds its retained scope',code='accounting_valuation_corrupt')
            current['quantity_msat']-=allocation['quantity_msat']
            current['adjustment_minor']-=allocation['released_minor']
        for carry in plan['carry']:
            key=(carry['valuation_id'],carry['asset'],carry['lot_id'])
            if key in states:
                raise AppError('Duplicate book adjustment carry destination',code='accounting_valuation_corrupt')
            states[key]=dict(quantity_msat=carry['quantity_msat'],adjustment_minor=carry['adjustment_minor'],
                             initial_quantity_msat=carry['quantity_msat'],initial_adjustment_minor=carry['adjustment_minor'],
                             available_after=carry['available_after'],valuation_digest=carry['valuation_digest'])
    return states,valuations,plans


def create_valuation(conn,profile_id,*,policy_id,artifact_id,period_id,effective_date,asset,adjustment_minor,
                     evidence_id,offset_account,valuation_kind,reason,idempotency_key):
    from .projection import get_policy,quantity_rows,_account
    with ledger.atomic(conn):
        book=ledger.require_book(conn,profile_id)
        effective_date=ledger._date(effective_date)
        asset=ledger._text(asset,'asset',maximum=16)
        ledger.strict_minor(abs(adjustment_minor)) if type(adjustment_minor) is int else ledger.strict_minor(adjustment_minor)
        if not adjustment_minor or valuation_kind not in ('impairment','write_back','revaluation') or (valuation_kind=='impairment' and adjustment_minor>0) or (valuation_kind=='write_back' and adjustment_minor<0):
            raise AppError('Review the signed book-only valuation adjustment',code='accounting_valuation_amount')
        reason=ledger._text(reason,'reason',maximum=2000)
        idempotency_key=ledger._text(idempotency_key,'idempotency_key',maximum=128)
        request=dict(policy_id=policy_id,artifact_id=artifact_id,period_id=period_id,effective_date=effective_date,asset=asset,
            adjustment_minor=adjustment_minor,evidence_id=evidence_id,offset_account=offset_account,valuation_kind=valuation_kind,
            reason=reason,idempotency_key=idempotency_key)
        existing=ledger._row(conn,'SELECT id FROM gl_book_valuations WHERE profile_id=? AND idempotency_key=?',(profile_id,idempotency_key))
        if existing:
            saved=get_valuation(conn,profile_id,existing['id'])
            if saved['valuation']['request']!=request:
                raise AppError('Valuation retry changed',code='accounting_idempotency_conflict')
            return saved
        period=ledger._period(conn,profile_id,period_id,open_required=True)
        if not period['start_date']<=effective_date<=period['end_date']:
            raise AppError('Valuation date is outside the open fiscal period',code='accounting_valuation_date')
        cutoff=datetime.combine(date.fromisoformat(effective_date)+timedelta(days=1),time.min,ZoneInfo(book['timezone'])).astimezone(timezone.utc).isoformat().replace('+00:00','Z')
        capture=require_calculation_current(conn,profile_id,artifact_id)['capture']
        if capture['cutoff_exclusive_utc']!=cutoff:
            raise AppError('Valuation requires a retained end-of-day calculation',code='accounting_valuation_date')
        policy=get_policy(conn,profile_id,policy_id)['policy']
        account=policy['asset_accounts'].get(asset)
        if policy['period_id']!=period_id or not account:
            raise AppError('Review the valuation policy and asset account',code='accounting_projection_policy')
        _account(conn,profile_id,offset_account,('expense',) if valuation_kind=='impairment' else ('expense','income','equity'))
        if account==offset_account:
            raise AppError('Valuation needs a distinct offset account',code='accounting_projection_account')
        evidence=ledger._row(conn,'SELECT id,content_sha256 FROM gl_evidence WHERE profile_id=? AND id=?',(profile_id,evidence_id))
        if not evidence:
            raise AppError('Valuation requires retained evidence from this book',code='accounting_evidence_required')
        result=next((item for item in capture['assets'] if item['asset']==asset),None)
        lots={item['lot_id']:item['quantity_msat'] for item in (result or {}).get('open_positions',[]) if item['quantity_msat']}
        if not lots:
            raise AppError('Valuation needs a positive current RP2 quantity scope',code='accounting_valuation_scope')
        have=sum(item['quantity_msat'] for p in _published_proposals(conn,profile_id,effective_date) for item in quantity_rows(p)
                 if item['asset']==asset and item['location']=='inventory')
        if have!=sum(lots.values()):
            raise AppError('Post and reconcile the source quantities before valuing them',code='accounting_valuation_scope')
        if conn.execute('''SELECT 1 FROM gl_entries e JOIN gl_lines l ON l.entry_id=e.id AND l.profile_id=e.profile_id
            WHERE e.profile_id=? AND e.status='posted' AND e.entry_date>? AND l.account_code=?''',(profile_id,effective_date,account)).fetchone():
            raise AppError('Later asset postings need correction before a backdated valuation',code='accounting_valuation_later_activity')
        balance=sum(row[0]-row[1] for row in conn.execute('''SELECT l.debit_minor,l.credit_minor FROM gl_lines l JOIN gl_entries e
            ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE l.profile_id=? AND l.account_code=? AND e.status='posted' AND e.entry_date<=?''',(profile_id,account,effective_date)))
        if balance+adjustment_minor<0:
            raise AppError('Valuation cannot make the asset carrying balance negative',code='accounting_valuation_amount')
        allocation=_allocate(adjustment_minor,lots)
        _validate_lot_amounts(conn,profile_id,effective_date,asset,result['open_positions'],allocation,
                             valuation_kind,book['minor_unit_exponent'])
        scope=[dict(lot_id=lot,quantity_msat=quantity,adjustment_minor=allocation[lot]) for lot,quantity in sorted(lots.items())]
        identity=uuid4().hex
        movements={account:adjustment_minor,offset_account:-adjustment_minor}
        draft=ledger.create_draft(conn,profile_id,dict(period_id=period_id,entry_date=effective_date,
            description=f'Reviewed book valuation: {asset}',source_ref=f'valuation:{identity}',idempotency_key=f'valuation:{identity}',
            lines=[dict(account_code=code,debit_minor=max(amount,0),credit_minor=max(-amount,0)) for code,amount in sorted(movements.items())]))
        payload=dict(request=request,scope_lots=scope,evidence_digest=evidence['content_sha256'],draft_digest=draft['payload_digest'],
            basis_authority='retained_rp2_lot_quantities',legal_approval=False,tax_adjustment_minor=0)
        checksum=ledger.digest(payload)
        conn.execute('INSERT INTO gl_book_valuations VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',(identity,profile_id,policy_id,artifact_id,period_id,
            effective_date,asset,account,evidence_id,draft['id'],ledger.canonical_json(payload),checksum,idempotency_key,now_iso()))
        ledger._bump(conn,profile_id)
        return get_valuation(conn,profile_id,identity)


def _validate_lot_amounts(conn,profile_id,effective_date,asset,positions,allocation,kind,exponent):
    """Never subsidize a negative lot with another lot's positive carrying value.

    A write-back restores earlier book impairments only. A reviewed revaluation
    is a separate explicit operation and is never presented as a legal approval.
    """
    states,_,_=_state(conn,profile_id,effective_date)
    existing={}
    for (_,state_asset,lot_id),state in states.items():
        if state_asset==asset:
            existing[lot_id]=existing.get(lot_id,0)+state['adjustment_minor']
    scale=Decimal(10)**exponent
    for position in positions:
        lot=position['lot_id']
        amount=allocation.get(lot,0)
        prior=existing.get(lot,0)
        if Decimal(position['basis_exact'])*scale+prior+amount<0:
            raise AppError('Quantity-proportional adjustment would make an individual lot negative',
                           code='accounting_valuation_lot_amount')
        if kind=='write_back' and amount>max(-prior,0):
            raise AppError('Write-back cannot exceed the remaining impairment of an individual lot',
                           code='accounting_valuation_write_back_ceiling')


def validate_draft(conn,profile_id,entry):
    row=ledger._row(conn,'SELECT id FROM gl_book_valuations WHERE profile_id=? AND draft_id=?',(profile_id,entry['id']))
    if row:
        valuation=get_valuation(conn,profile_id,row['id'])
        capture=require_calculation_current(conn,profile_id,valuation['artifact_id'])['capture']
        if valuation['voided'] or valuation['valuation']['draft_digest']!=entry['payload_digest']:
            raise AppError('Valuation draft approval changed',code='accounting_stale_approval')
        ledger._period(conn,profile_id,valuation['period_id'],open_required=True)
        if conn.execute('''SELECT 1 FROM gl_entries e JOIN gl_lines l ON l.entry_id=e.id AND l.profile_id=e.profile_id
            WHERE e.profile_id=? AND e.status='posted' AND e.entry_date>? AND l.account_code=?''',
            (profile_id,valuation['effective_date'],valuation['account_code'])).fetchone():
            raise AppError('Later asset postings require replacement of this valuation draft',code='accounting_valuation_later_activity')
        balance=sum(row[0]-row[1] for row in conn.execute('''SELECT l.debit_minor,l.credit_minor FROM gl_lines l JOIN gl_entries e
            ON e.id=l.entry_id AND e.profile_id=l.profile_id WHERE l.profile_id=? AND l.account_code=? AND e.status='posted' AND e.entry_date<=?''',
            (profile_id,valuation['account_code'],valuation['effective_date'])))
        if balance+valuation['valuation']['request']['adjustment_minor']<0:
            raise AppError('Valuation would make the carrying balance negative',code='accounting_valuation_amount')
        result=next(item for item in capture['assets'] if item['asset']==valuation['asset'])
        _validate_lot_amounts(conn,profile_id,valuation['effective_date'],valuation['asset'],result['open_positions'],
            {item['lot_id']:item['adjustment_minor'] for item in valuation['valuation']['scope_lots']},
            valuation['valuation']['request']['valuation_kind'],ledger.require_book(conn,profile_id)['minor_unit_exponent'])


def after_post(conn,profile_id,entry_id):
    row=ledger._row(conn,'SELECT id FROM gl_book_valuations WHERE profile_id=? AND draft_id=?',(profile_id,entry_id))
    if row and not conn.execute('SELECT 1 FROM gl_valuation_publications WHERE valuation_id=?',(row['id'],)).fetchone():
        conn.execute('INSERT INTO gl_valuation_publications VALUES(?,?,?,?)',(row['id'],profile_id,entry_id,now_iso()))


def post_valuation(conn,profile_id,*,valuation_id,expected_digest):
    with ledger.atomic(conn):
        valuation=get_valuation(conn,profile_id,valuation_id)
        if expected_digest!=valuation['payload_digest']:
            raise AppError('Valuation approval changed',code='accounting_stale_approval')
        if valuation['published']:
            return valuation
        validate_draft(conn,profile_id,ledger._entry(conn,profile_id,valuation['draft_id']))
        ledger.post_draft(conn,profile_id,draft_id=valuation['draft_id'],expected_digest=valuation['valuation']['draft_digest'])
        after_post(conn,profile_id,valuation['draft_id'])
        return get_valuation(conn,profile_id,valuation_id)


def require_reversible(conn,profile_id,entry_id):
    row=ledger._row(conn,'SELECT id FROM gl_book_valuations WHERE profile_id=? AND draft_id=?',(profile_id,entry_id))
    if not row:
        return
    current=get_valuation(conn,profile_id,row['id'])
    if conn.execute('''SELECT 1 FROM gl_book_valuations later JOIN gl_valuation_publications publication
        ON publication.valuation_id=later.id WHERE later.profile_id=? AND later.asset=?
        AND publication.rowid>(SELECT rowid FROM gl_valuation_publications WHERE valuation_id=?)
        AND NOT EXISTS(SELECT 1 FROM gl_valuation_voids v WHERE v.valuation_id=later.id)''',
        (profile_id,current['asset'],row['id'])).fetchone():
        raise AppError('Reverse subsequent valuation layers for this asset first',code='accounting_valuation_in_use')
    _,_,plans=_state(conn,profile_id,'9999-12-31')
    if any(allocation['valuation_id']==row['id'] for record in plans for allocation in record['plan']['allocations']):
        raise AppError('Reverse dependent source postings before reversing this valuation',code='accounting_valuation_in_use')


def after_reverse(conn,profile_id,original_id,reversal_id):
    row=ledger._row(conn,'SELECT id FROM gl_book_valuations WHERE profile_id=? AND draft_id=?',(profile_id,original_id))
    if row:
        reversal=ledger._entry(conn,profile_id,reversal_id)
        conn.execute('INSERT INTO gl_valuation_voids VALUES(?,?,?,?,?,?)',(row['id'],profile_id,reversal_id,reversal['entry_date'],'Posted valuation reversed',now_iso()))


def before_discard(conn,profile_id,entry):
    row=ledger._row(conn,'SELECT id FROM gl_book_valuations WHERE profile_id=? AND draft_id=?',(profile_id,entry['id']))
    if row:
        conn.execute('INSERT INTO gl_valuation_voids VALUES(?,?,?,?,?,?)',(row['id'],profile_id,None,entry['entry_date'],'Valuation draft discarded',now_iso()))


def controls(conn,profile_id,end_date):
    states,valuations,plans=_state(conn,profile_id,end_date)
    remaining,original,accounts={},{},{}
    for (_,asset,_),state in states.items():
        remaining[asset]=remaining.get(asset,0)+state['adjustment_minor']
    for record in valuations:
        amount=record['valuation']['request']['adjustment_minor']
        original[record['asset']]=original.get(record['asset'],0)+amount
        accounts[record['account_code']]=accounts.get(record['account_code'],0)+amount
    drafts=[]
    if _enabled(conn):
        drafts=[dict(code='accounting_valuation_unpublished',valuation_id=row[0]) for row in conn.execute('''SELECT v.id FROM gl_book_valuations v
            WHERE v.profile_id=? AND v.effective_date<=? AND NOT EXISTS(SELECT 1 FROM gl_valuation_publications x WHERE x.valuation_id=v.id)
            AND NOT EXISTS(SELECT 1 FROM gl_valuation_voids z WHERE z.valuation_id=v.id)''',(profile_id,end_date))]
    return dict(remaining_by_asset_minor=remaining,original_by_asset_minor=original,account_adjustments_minor=accounts,
        valuations=valuations,release_plans=plans,blockers=drafts,tax_adjustment_minor=0)


def list_valuations(conn,profile_id,*,period_id,limit=100,cursor=None):
    from .projection_views import _page
    with ledger.atomic(conn):
        ledger._period(conn,profile_id,period_id)
        rows=[get_valuation(conn,profile_id,row[0]) for row in conn.execute(
            'SELECT id FROM gl_book_valuations WHERE profile_id=? AND period_id=? ORDER BY effective_date,id',(profile_id,period_id))]
        return _page(conn,profile_id,dict(kind='valuations',period_id=period_id),rows,limit,cursor)
