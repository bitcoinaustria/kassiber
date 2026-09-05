"""Exact book-currency representation, independent of RP2's basis arithmetic.

Each asset/transit account represents the HALF_EVEN-rounded cumulative exact
book value. A new movement takes the difference from the already represented
value, so repeated sub-cent disposals cannot leave a phantom carrying balance.
The retained policy assigns the explicit remainder to its gains/losses account.
"""
from datetime import datetime
from decimal import Decimal, localcontext

from ...errors import AppError
from . import ledger


def apply_to_event(conn,profile_id,event,exact,lines,policy):
    from .artifacts import get_calculation
    from .projection import _minor,get_proposal,quantity_rows
    with localcontext() as context:
        context.prec=32
        postings=[{key:value for key,value in exact.items() if key!='related_postings'},
                  *exact.get('related_postings',[])]
        codes={item['account_code'] for item in postings}
        previous={code:dict(basis=Decimal(0),minor=0,dependencies=[]) for code in codes}
        for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p JOIN gl_projection_publications x
            ON x.proposal_id=p.id WHERE p.profile_id=? AND NOT EXISTS(
                SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id) ORDER BY x.rowid''',(profile_id,)):
            saved=get_proposal(conn,profile_id,row[0])
            relevant=[item for item in quantity_rows(saved) if item['account_code'] in codes]
            if not relevant:
                continue
            capture=get_calculation(conn,profile_id,saved['artifact_id'])['capture']
            saved_event=next((item for item in capture['inputs']['prepared_transactions'] if item['unique_id']==saved['event_id']),None)
            if saved_event and datetime.fromisoformat(saved_event['timestamp'])>datetime.fromisoformat(event['timestamp']):
                raise AppError('Correct later source postings before inserting earlier book movements',
                               code='accounting_projection_later_activity')
            for item in relevant:
                state=previous[item['account_code']]
                state['basis']+=Decimal(item['basis_exact'])
                state['minor']+=item['book_value_minor']
                state['dependencies'].append((saved['id'],saved['payload_digest']))
        movements={line['account_code']:line['debit_minor']-line['credit_minor'] for line in lines}
        rounding=[]
        exponent=ledger.require_book(conn,profile_id)['minor_unit_exponent']
        for item in postings:
            state=previous[item['account_code']]
            value=_minor(state['basis']+Decimal(item['basis_exact']),exponent)-state['minor']
            remainder=value-item['book_value_minor']
            movements[item['account_code']]=movements.get(item['account_code'],0)+remainder
            movements[policy['gain_account']]=movements.get(policy['gain_account'],0)-remainder
            rounding.append(dict(account_code=item['account_code'],before_basis_exact=format(state['basis'],'f'),
                before_minor=state['minor'],unrounded_event_minor=item['book_value_minor'],remainder_minor=remainder,
                dependencies_digest=ledger.digest(state['dependencies'])))
            item['book_value_minor']=value
            state['basis']+=Decimal(item['basis_exact'])
            state['minor']+=value
        result={**postings[0],'currency_rounding':rounding}
        if len(postings)>1:
            result['related_postings']=postings[1:]
        lines=[dict(account_code=code,debit_minor=max(amount,0),credit_minor=max(-amount,0))
               for code,amount in sorted(movements.items()) if amount]
        return result,lines


def require_reversible(conn,profile_id,proposal_id):
    from .projection import get_proposal,quantity_rows
    current=get_proposal(conn,profile_id,proposal_id)
    codes={item['account_code'] for item in quantity_rows(current)}
    for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p JOIN gl_projection_publications x
        ON x.proposal_id=p.id WHERE p.profile_id=? AND x.rowid>(
            SELECT rowid FROM gl_projection_publications WHERE proposal_id=?)
        AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''',(profile_id,proposal_id)):
        if any(item['account_code'] in codes for item in quantity_rows(get_proposal(conn,profile_id,row[0]))):
            raise AppError('Reverse dependent later book-currency movements first',code='accounting_projection_rounding_in_use')
