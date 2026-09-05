"""Reviewed opening migration using retained RP2 quantities and basis."""
from datetime import date, datetime, time, timezone
from decimal import Decimal, ROUND_HALF_EVEN, localcontext
from uuid import uuid4
from zoneinfo import ZoneInfo

from ...errors import AppError
from ...time_utils import now_iso
from . import ledger, projection, sources
from .artifacts import require_calculation_current


def _inputs(conn,profile_id,artifact_id,period_id):
    book=ledger.require_book(conn,profile_id)
    period=ledger._period(conn,profile_id,period_id,open_required=True)
    artifact=require_calculation_current(conn,profile_id,artifact_id)
    cutoff=datetime.combine(date.fromisoformat(period['start_date']),time.min,ZoneInfo(book['timezone'])).astimezone(timezone.utc).isoformat().replace('+00:00','Z')
    if artifact['capture']['cutoff_exclusive_utc']!=cutoff:
        raise AppError('Opening requires a calculation immediately before the fiscal start',code='accounting_projection_period')
    snapshot=sources.get_snapshot(conn,profile_id,artifact['source_snapshot_id'])
    records=[item for item in snapshot['snapshot']['sources'] if item['kind']=='custody' and item['occurred_on']<period['start_date'] and item['amount_atomic']]
    claims=[dict(source_id=item['source_id'],start_atomic=0,end_atomic=item['amount_atomic']) for item in records]
    return book,period,artifact,snapshot,claims


def preview_opening(conn,profile_id,*,artifact_id,period_id):
    _,period,artifact,snapshot,claims=_inputs(conn,profile_id,artifact_id,period_id)
    return dict(artifact_id=artifact_id,period_id=period_id,entry_date=period['start_date'],
        snapshot_id=snapshot['id'],source_digest=snapshot['input_digest'],source_count=len(claims),
        assets=artifact['capture']['assets'],pending_transfers=artifact['capture']['inputs'].get('cutoff_relations',[]))


def bind_opening_sources(conn,profile_id,*,artifact_id,period_id,expected_source_digest,reason,idempotency_key):
    with ledger.atomic(conn):
        _,_,_,snapshot,claims=_inputs(conn,profile_id,artifact_id,period_id)
        if not claims:
            raise AppError('No historical Bitcoin sources need an opening migration',code='accounting_projection_event')
        return sources.bind_sources(conn,profile_id,snapshot_id=snapshot['id'],expected_digest=expected_source_digest,
            economic_id=f'opening:{period_id}',role='recognition',claims=claims,reason=reason,idempotency_key=idempotency_key)


def validate_binding(conn,profile_id,proposal):
    request=proposal['proposal']['request']
    _,_,_,snapshot,expected=_inputs(conn,profile_id,proposal['artifact_id'],proposal['period_id'])
    binding=sources.get_binding(conn,profile_id,proposal['binding_id'])
    actual=[{key:item[key] for key in ('source_id','start_atomic','end_atomic')} for item in binding['claims']]
    if binding['voided'] or binding['role']!='recognition' or binding['snapshot_id']!=snapshot['id'] or sorted(actual,key=lambda row:row['source_id'])!=sorted(expected,key=lambda row:row['source_id']):
        raise AppError('Opening must cover its full reviewed historical source population',code='accounting_projection_binding')
    if request['category']!='opening':
        raise AppError('Invalid opening proposal',code='accounting_projection_event')


def create_opening_proposal(conn,profile_id,*,policy_id,artifact_id,binding_id,period_id,idempotency_key,additional_balances=None):
    with ledger.atomic(conn),localcontext() as context:
        context.prec=32
        context.rounding=ROUND_HALF_EVEN
        balances=[] if additional_balances is None else additional_balances
        if not isinstance(balances,list) or len(balances)>500:
            raise AppError('Opening balances require a bounded account list',code='accounting_validation')
        request=dict(category='opening',policy_id=policy_id,artifact_id=artifact_id,binding_id=binding_id,
            period_id=period_id,idempotency_key=idempotency_key,additional_balances=balances)
        existing=ledger._row(conn,'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND idempotency_key=?',(profile_id,idempotency_key))
        if existing:
            saved=projection.get_proposal(conn,profile_id,existing['id'])
            if saved['proposal']['request']!=request:
                raise AppError('Opening retry changed',code='accounting_idempotency_conflict')
            return saved
        book,period,artifact,_,_=_inputs(conn,profile_id,artifact_id,period_id)
        policy=projection.get_policy(conn,profile_id,policy_id)['policy']
        if policy['period_id']!=period_id:
            raise AppError('Review the opening policy for this fiscal period',code='accounting_projection_policy')
        postings=[]
        def position(asset,quantity,basis,location,relation_id=None):
            account=policy['transit_accounts' if location=='transit' else 'asset_accounts'].get(asset)
            if not account:
                raise AppError('Opening asset/transit account is missing',code='accounting_projection_account')
            prior=[item for item in postings if item['account_code']==account]
            represented=projection._minor(sum((Decimal(item['basis_exact']) for item in prior),Decimal(0))+basis,
                                           book['minor_unit_exponent'])-sum(item['book_value_minor'] for item in prior)
            postings.append(dict(asset=asset,quantity_msat=quantity,basis_exact=format(basis,'f'),location=location,
                account_code=account,book_value_minor=represented,**({'relation_id':relation_id} if relation_id else {})))
        for asset in artifact['capture']['assets']:
            quantity=sum(item['quantity_msat'] for item in asset['open_positions'])
            basis=sum((Decimal(item['basis_exact']) for item in asset['open_positions']),Decimal(0))
            if quantity or basis:
                position(asset['asset'],quantity,basis,'inventory')
        for relation in artifact['capture']['inputs'].get('cutoff_relations',[]):
            if relation.get('future_reference'):
                position(relation['to_asset'],relation['quantity_received_msat'],Decimal(relation['basis_carried_exact']),'transit',relation['relation_id'])
        if not postings:
            raise AppError('No Bitcoin opening position remains',code='accounting_projection_event')
        movements={}
        for item in postings:
            movements[item['account_code']]=movements.get(item['account_code'],0)+item['book_value_minor']
        forbidden=set(policy['asset_accounts'].values())|set(policy['transit_accounts'].values())|{policy['capital_account']}
        seen=set()
        for balance in balances:
            if not isinstance(balance,dict) or set(balance)!={'account_code','balance_minor'}:
                raise AppError('Opening balance requires account and exact signed minor units',code='accounting_validation')
            code=projection._account(conn,profile_id,balance['account_code'],('asset','liability','equity'))
            amount=balance['balance_minor']
            if type(amount) is not int:
                raise AppError('Opening amount requires exact integer minor units',code='accounting_validation')
            ledger.strict_minor(abs(amount))
            if code in forbidden or code in seen:
                raise AppError('Opening account is duplicated or already derived',code='accounting_projection_account')
            movements[code]=amount
            seen.add(code)
        movements[policy['capital_account']]=-sum(movements.values())
        lines=[dict(account_code=code,debit_minor=max(amount,0),credit_minor=max(-amount,0)) for code,amount in sorted(movements.items()) if amount]
        identity=uuid4().hex
        draft=ledger.create_draft(conn,profile_id,dict(period_id=period_id,entry_date=period['start_date'],entry_kind='opening',
            description='Reviewed opening balances',source_ref=f'projection:{identity}',idempotency_key=f'projection:{identity}',lines=lines)) if lines else None
        main={**postings[0],'related_postings':postings[1:]}
        payload=dict(request=request,quantitative_posting=main,lines=lines,draft_digest=draft['payload_digest'] if draft else None,policy_digest=ledger.digest(policy))
        checksum=ledger.digest(payload)
        conn.execute('INSERT INTO gl_projection_proposals VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)',(identity,profile_id,policy_id,artifact_id,binding_id,
            f'opening:{period_id}',period_id,period['start_date'],draft['id'] if draft else None,ledger.canonical_json(payload),checksum,idempotency_key,now_iso()))
        result=projection.get_proposal(conn,profile_id,identity)
        validate_binding(conn,profile_id,result)
        ledger._bump(conn,profile_id)
        return result
