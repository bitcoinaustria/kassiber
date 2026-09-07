"""Release reviewed book adjustments along RP2-selected quantity fragments."""
from datetime import datetime
from decimal import Decimal
import json
from zoneinfo import ZoneInfo

from ...errors import AppError
from ...time_utils import now_iso
from . import ledger, valuation


def _ratio(amount,numerator,denominator):
    quotient,remainder=divmod(abs(amount)*numerator,denominator)
    if remainder*2>denominator or (remainder*2==denominator and quotient%2):
        quotient+=1
    return -quotient if amount<0 else quotient


def plan_event(conn,profile_id,capture,event_id,event,category):
    if not valuation._enabled(conn) or category in ('purchase','income','capital','opening','transfer_receipt'):
        return None
    day=datetime.fromisoformat(event['timestamp']).astimezone(ZoneInfo(ledger.require_book(conn,profile_id)['timezone'])).date().isoformat()
    states,_,existing=valuation._state(conn,profile_id,'9999-12-31')
    for record in existing:
        if record['plan']['asset']==event['asset'] and datetime.fromisoformat(record['plan']['timestamp'])>datetime.fromisoformat(event['timestamp']):
            raise AppError('Later adjustment releases require correction before this source event',code='accounting_valuation_later_activity')
    execution=[item for item in capture['inputs'].get('execution_basis',[]) if item['asset']==event['asset'] and item['event_id']==event_id]
    principal=0 if event['kind']=='INTRA' else int(Decimal(event['crypto_out_no_fee_exact'])*100_000_000_000)
    lots={}
    for fragment in execution:
        used=min(principal,fragment['quantity_msat'])
        principal-=used
        lot=lots.setdefault(fragment['lot_id'],dict(quantity_msat=0,principal_msat=0))
        lot['quantity_msat']+=fragment['quantity_msat']
        lot['principal_msat']+=used
    allocations=[]
    for (valuation_id,asset,lot_id),state in sorted(states.items()):
        if asset!=event['asset'] or lot_id not in lots or not state['quantity_msat']:
            continue
        available=state['available_after']
        if ('T' in available and datetime.fromisoformat(available)>datetime.fromisoformat(event['timestamp'])) or ('T' not in available and available>=day):
            continue
        consumed=lots[lot_id]['quantity_msat']
        if consumed>state['quantity_msat']:
            raise AppError('RP2 consumption exceeds the remaining reviewed valuation scope',code='accounting_valuation_scope')
        cumulative=state['initial_quantity_msat']-state['quantity_msat']+consumed
        desired=_ratio(state['initial_adjustment_minor'],cumulative,state['initial_quantity_msat'])
        released=desired-(state['initial_adjustment_minor']-state['adjustment_minor'])
        principal_minor=_ratio(released,lots[lot_id]['principal_msat'],consumed)
        allocations.append(dict(valuation_id=valuation_id,asset=asset,lot_id=lot_id,quantity_msat=consumed,
            valuation_digest=state['valuation_digest'],
            released_minor=released,principal_minor=principal_minor,fee_minor=released-principal_minor,
            before_quantity_msat=state['quantity_msat'],before_adjustment_minor=state['adjustment_minor']))
    if not allocations:
        return None
    carry=[]
    if category=='transfer_dispatch':
        relation=next(item for item in capture['inputs']['custody_relations'] if item.get('out_id')==event_id)
        incoming=next(item for item in capture['inputs']['prepared_transactions'] if item['unique_id']==relation['in_id'])
        by_valuation={}
        for item in allocations:
            by_valuation[item['valuation_id']]=by_valuation.get(item['valuation_id'],0)+item['principal_minor']
        for identity,amount in sorted(by_valuation.items()):
            # One RP2 incoming lot merges the source principal. The book-only
            # adjustment follows that same new lot, not a synthetic tax lot.
            if amount:
                carry.append(dict(valuation_id=identity,asset=incoming['asset'],lot_id=incoming['unique_id'],
                    quantity_msat=int(Decimal(incoming['crypto_in_exact'])*100_000_000_000),adjustment_minor=amount,
                    available_after=incoming['timestamp'],valuation_digest=next(item['valuation_digest'] for item in allocations if item['valuation_id']==identity)))
    return dict(event_id=event_id,asset=event['asset'],timestamp=event['timestamp'],allocations=allocations,carry=carry)


def apply_to_event(conn,profile_id,capture,event_id,event,exact,lines,policy,category):
    scale=Decimal(10)**ledger.require_book(conn,profile_id)['minor_unit_exponent']
    if category=='transfer_receipt':
        relation=next(item for item in capture['inputs']['custody_relations'] if item.get('in_id')==event_id)
        from ._projection_events import published_transit_origin
        origin=published_transit_origin(conn,profile_id,relation)
        if origin:
            _,transit=origin
            adjusted=transit['book_value_minor']-exact['related_postings'][0]['book_value_minor']
            if adjusted:
                exact={**exact,'basis_exact':format(-Decimal(transit['basis_exact']),'f'),
                    'book_value_minor':-transit['book_value_minor'],'book_adjustment_minor':-adjusted,
                    'related_postings':[{**exact['related_postings'][0],'basis_exact':transit['basis_exact'],
                        'book_value_minor':transit['book_value_minor'],'book_adjustment_minor':adjusted}]}
                movements={exact['account_code']:-transit['book_value_minor'],exact['related_postings'][0]['account_code']:transit['book_value_minor']}
                lines=[dict(account_code=code,debit_minor=max(amount,0),credit_minor=max(-amount,0)) for code,amount in sorted(movements.items()) if amount]
        return exact,lines,None
    plan=plan_event(conn,profile_id,capture,event_id,event,category)
    if not plan:
        return exact,lines,None
    total=sum(item['released_minor'] for item in plan['allocations'])
    principal=sum(item['principal_minor'] for item in plan['allocations'])
    fee=total-principal
    movements={line['account_code']:line['debit_minor']-line['credit_minor'] for line in lines}
    def add(code,amount):
        movements[code]=movements.get(code,0)+amount
    add(exact['account_code'],-total)
    exact={**exact,'basis_exact':format(Decimal(exact['basis_exact'])-Decimal(total)/scale,'f'),
        'book_value_minor':exact['book_value_minor']-total,'book_adjustment_minor':-total}
    if category=='transfer_dispatch':
        other=exact['related_postings'][0]
        add(other['account_code'],principal)
        add(policy['fee_account'],fee)
        exact['related_postings']=[{**other,'basis_exact':format(Decimal(other['basis_exact'])+Decimal(principal)/scale,'f'),
            'book_value_minor':other['book_value_minor']+principal,'book_adjustment_minor':principal}]
    elif category=='custody_move':
        add(policy['fee_account'],total)
    else:
        add(policy['gain_account'],principal)
        add(policy['fee_account'],fee)
    lines=[dict(account_code=code,debit_minor=max(amount,0),credit_minor=max(-amount,0)) for code,amount in sorted(movements.items()) if amount]
    return exact,lines,plan


def retain_plan(conn,profile_id,proposal_id,plan):
    if plan is not None:
        conn.execute('INSERT INTO gl_valuation_releases VALUES(?,?,?,?,?)',
            (proposal_id,profile_id,ledger.canonical_json(plan),ledger.digest(plan),now_iso()))


def validate_proposal(conn,profile_id,proposal,capture):
    if not valuation._enabled(conn):
        return
    from .projection import get_policy,_event
    request=proposal['proposal']['request']
    if request['category']=='opening':
        return
    book=ledger.require_book(conn,profile_id)
    policy=get_policy(conn,profile_id,proposal['policy_id'])['policy']
    event,exact,lines=_event(capture,proposal['event_id'],policy,request['category'],book['minor_unit_exponent'])
    current,_,plan=apply_to_event(conn,profile_id,capture,proposal['event_id'],event,exact,lines,policy,request['category'])
    from .currency_rounding import apply_to_event as round_currency
    current,_=round_currency(conn,profile_id,event,current,lines,policy)
    saved=ledger._row(conn,'SELECT payload_json FROM gl_valuation_releases WHERE profile_id=? AND proposal_id=?',(profile_id,proposal['id']))
    if (json.loads(saved['payload_json']) if saved else None)!=plan or current!=proposal['proposal']['quantitative_posting']:
        raise AppError('Book valuation scope changed; replace and review this draft',code='accounting_valuation_stale_release')


def require_proposal_reversible(conn,profile_id,proposal_id):
    from .currency_rounding import require_reversible
    require_reversible(conn,profile_id,proposal_id)
    if not valuation._enabled(conn):
        return
    _,_,plans=valuation._state(conn,profile_id,'9999-12-31')
    position=next((index for index,item in enumerate(plans) if item['proposal_id']==proposal_id),None)
    if position is None:
        return
    valuation_ids={item['valuation_id'] for item in plans[position]['plan']['allocations']}
    if any(item['valuation_id'] in valuation_ids for following in plans[position+1:] for item in following['plan']['allocations']):
        raise AppError('Reverse later dependent book-adjustment releases first',code='accounting_valuation_in_use')
