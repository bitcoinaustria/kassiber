"""Book movement arithmetic over retained authoritative transfer results."""
from decimal import Decimal

from ...errors import AppError


def published_transit_origin(conn,profile_id,relation):
    """Resolve exact retained dispatch authority, including pre-book migration."""
    from . import ledger
    from .artifacts import get_calculation
    from .projection import get_proposal,quantity_rows
    for row in conn.execute('''SELECT p.id FROM gl_projection_proposals p JOIN gl_projection_publications x
        ON x.proposal_id=p.id WHERE p.profile_id=? AND NOT EXISTS(
            SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id) ORDER BY x.rowid''',(profile_id,)):
        proposal=get_proposal(conn,profile_id,row[0])
        category=proposal['proposal']['request']['category']
        if category=='transfer_dispatch' and proposal['event_id']==relation['out_id']:
            return proposal,next(item for item in quantity_rows(proposal) if item['location']=='transit')
        if category!='opening':
            continue
        capture=get_calculation(conn,profile_id,proposal['artifact_id'])['capture']
        retained=next((item for item in capture['inputs'].get('cutoff_relations',[]) if item.get('future_reference')
            and item['out_id']==relation['out_id'] and item['in_id']==relation['in_id']
            and item['relation_id']==relation['pair_id']),None)
        if retained:
            position=next((item for item in quantity_rows(proposal) if item['location']=='transit'
                and item.get('relation_id')==retained['relation_id'] and item['asset']==retained['to_asset']
                and item['quantity_msat']==retained['quantity_received_msat']
                and Decimal(item['basis_exact'])==Decimal(retained['basis_carried_exact'])),None)
            if position:
                return proposal,position
    return None


def same_asset_event(capture,event_id,policy,exponent):
    from .projection import _minor
    move=next((item for item in capture['inputs'].get('same_asset_moves',[])
               if item.get('rp2_unique_id',item['out_id'])==event_id),None)
    event=next((item for item in capture['inputs']['prepared_transactions'] if item['unique_id']==event_id and item['kind']=='INTRA'),None)
    if not move or not event:
        raise AppError('Select an executed same-asset custody move',code='accounting_projection_event')
    account=policy['asset_accounts'].get(move['asset'])
    if not account:
        raise AppError('Asset account is missing',code='accounting_projection_account')
    execution=[item for item in capture['inputs']['execution_basis'] if item['asset']==move['asset'] and item['event_id']==event_id]
    fee=move['crypto_fee_msat']
    if sum(item['quantity_msat'] for item in execution)!=fee:
        raise AppError('Custody fee execution does not reconcile',code='accounting_projection_basis')
    basis=sum((Decimal(item['basis_exact']) for item in execution),Decimal(0))
    minor=_minor(basis,exponent)
    claims=[dict(event_id=move['out_id'],quantity_msat=move['crypto_sent_msat'])]
    # At-cutoff transit is synthetic custody, not an imported future receipt.
    if not move['in_id'].startswith('accounting-transit:'):
        claims.append(dict(event_id=move['in_id'],quantity_msat=move['crypto_received_msat']))
    exact=dict(asset=move['asset'],quantity_msat=-fee,basis_exact=format(-basis,'f'),account_code=account,
               location='inventory',book_value_minor=-minor,required_claims=claims,custody_move=move)
    lines=sorted([dict(account_code=account,debit_minor=0,credit_minor=minor),
                  dict(account_code=policy['fee_account'],debit_minor=minor,credit_minor=0)],key=lambda row:row['account_code']) if minor else []
    return event,exact,lines


def transfer_event(capture, event_id, policy, category, exponent):
    from .projection import _minor

    relations = capture['inputs'].get('custody_relations', [])
    relation = next((item for item in relations if item.get('out_id' if category == 'transfer_dispatch' else 'in_id') == event_id
                     and item.get('out_asset') != item.get('in_asset')), None)
    if not relation:
        raise AppError('Select a retained cross-asset custody relation', code='accounting_projection_event')
    result = next((item for asset in capture['assets'] for item in asset['transfers'] if item['event_id'] == relation['out_id']), None)
    event = next((item for item in capture['inputs']['prepared_transactions'] if item['unique_id'] == event_id), None)
    if not result or not event or event.get('execution_role'):
        raise AppError('The selected transfer leg has not executed by this cutoff', code='accounting_projection_event')
    source_asset, target_asset = result['from_asset'],result['to_asset']
    source = policy['asset_accounts'].get(source_asset)
    target = policy['asset_accounts'].get(target_asset)
    transit = policy['transit_accounts'].get(target_asset)
    if not source or not target or not transit:
        raise AppError('Review source, destination and transit accounts', code='accounting_projection_account')
    carried = Decimal(result['basis_carried_exact'])
    value = _minor(carried,exponent)
    received = result['quantity_received_msat']
    if category == 'transfer_receipt':
        movements = {transit:-value,target:value}
        main = dict(asset=target_asset,quantity_msat=-received,basis_exact=format(-carried,'f'),
                    account_code=transit,location='transit',book_value_minor=-value)
        related = dict(asset=target_asset,quantity_msat=received,basis_exact=format(carried,'f'),
                       account_code=target,location='inventory',book_value_minor=value)
        claim_quantity = received
    else:
        execution = [item for item in capture['inputs'].get('execution_basis', [])
                     if item['asset'] == source_asset and item['event_id'] == event_id]
        sent = result['quantity_sent_msat'] + result['fee_msat']
        if sum(item['quantity_msat'] for item in execution) != sent:
            raise AppError('Transfer source execution does not reconcile', code='accounting_projection_basis')
        consumed = sum(Decimal(item['basis_exact']) for item in execution)
        total = _minor(consumed,exponent)
        if consumed < carried:
            raise AppError('Transfer carrying basis exceeds source consumption', code='accounting_projection_basis')
        movements = {source:-total,transit:value,policy['fee_account']:total-value}
        main = dict(asset=source_asset,quantity_msat=-sent,basis_exact=format(-consumed,'f'),
                    account_code=source,location='inventory',book_value_minor=-total)
        related = dict(asset=target_asset,quantity_msat=received,basis_exact=format(carried,'f'),
                       account_code=transit,location='transit',book_value_minor=value)
        claim_quantity = sent
    main['related_postings'] = [related]
    main['required_claims'] = [dict(event_id=event_id,quantity_msat=claim_quantity)]
    main['relation_id'] = relation['pair_id']
    lines = [dict(account_code=code,debit_minor=max(amount,0),credit_minor=max(-amount,0))
             for code,amount in sorted(movements.items()) if amount]
    return event,main,lines
