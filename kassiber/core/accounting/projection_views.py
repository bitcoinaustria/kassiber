"""Bounded reviewed-source worklists for desktop and CLI."""
import base64
from datetime import datetime
import json
from zoneinfo import ZoneInfo

from ...errors import AppError
from . import ledger, projection
from .artifacts import get_calculation


def _page(conn,profile_id,scope,items,limit,cursor):
    book=ledger.require_book(conn,profile_id)
    if type(limit) is not int or not 1<=limit<=500:
        raise AppError('Page size must be between 1 and 500',code='accounting_validation')
    commitment=ledger.digest(dict(profile_id=profile_id,revision=book['revision'],scope=scope))
    start=0
    if cursor is not None:
        try:
            if not isinstance(cursor,str) or len(cursor)>4096:
                raise ValueError()
            token=json.loads(base64.b64decode(cursor,altchars=b'-_',validate=True))
            if set(token)!={'scope','start'} or type(token['start']) is not int or not 0<token['start']<len(items):
                raise ValueError()
            if token['scope']!=commitment:
                raise AppError('Projection worklist changed; restart the first page',code='accounting_stale_cursor')
            start=token['start']
        except (ValueError,TypeError,KeyError,json.JSONDecodeError) as exc:
            raise AppError('Invalid projection continuation',code='accounting_invalid_cursor') from exc
    end=min(start+limit,len(items))
    continuation=base64.urlsafe_b64encode(ledger.canonical_json(dict(scope=commitment,start=end)).encode()).decode() if end<len(items) else None
    return dict(rows=items[start:end],total_count=len(items),next_cursor=continuation,revision=book['revision'])


def list_events(conn,profile_id,*,artifact_id,period_id,limit=100,cursor=None):
    with ledger.atomic(conn):
        book=ledger.require_book(conn,profile_id)
        period=ledger._period(conn,profile_id,period_id)
        capture=get_calculation(conn,profile_id,artifact_id)['capture']
        relations=capture['inputs'].get('custody_relations',[])
        source_map=capture['inputs']['source_event_map']
        events=[]
        for item in capture['inputs']['prepared_transactions']:
            if item.get('execution_role'):
                continue
            day=datetime.fromisoformat(item['timestamp']).astimezone(ZoneInfo(book['timezone'])).date().isoformat()
            if not period['start_date']<=day<=period['end_date']:
                continue
            identity=item['unique_id']
            relation=next((relation for relation in relations if identity in (relation.get('out_id'),relation.get('in_id'))),None)
            claim_events=[identity]
            if item['kind']=='INTRA':
                categories=['custody_move']
                move=next((move for move in capture['inputs'].get('same_asset_moves',[]) if move.get('rp2_unique_id',move['out_id'])==identity),None)
                if not move:
                    continue
                claim_events=[move['out_id']]+([] if move['in_id'].startswith('accounting-transit:') else [move['in_id']])
                quantity=move['crypto_sent_msat']
            elif relation:
                categories=['transfer_dispatch' if identity==relation['out_id'] else 'transfer_receipt']
                from .projection import _amount_msat
                quantity=_amount_msat(item,'crypto_in_exact' if item['kind']=='IN' else 'crypto_out_with_fee_exact')
            else:
                categories=['purchase','income','capital'] if item['kind']=='IN' else ['disposal']
                from .projection import _amount_msat
                quantity=_amount_msat(item,'crypto_in_exact' if item['kind']=='IN' else 'crypto_out_with_fee_exact')
            claims=[]
            for event_id in claim_events:
                mapping=source_map.get(event_id,{})
                claims.extend(dict(source_id=mapping['source_id'],**slice_) for slice_ in mapping.get('claim_slices',[]) if mapping.get('source_id'))
            existing=ledger._row(conn,'''SELECT p.id FROM gl_projection_proposals p WHERE profile_id=? AND event_id=?
                AND NOT EXISTS(SELECT 1 FROM gl_projection_voids v WHERE v.proposal_id=p.id)''',(profile_id,identity))
            bindings=set()
            for claim in claims:
                bindings.update(row[0] for row in conn.execute('''SELECT c.binding_id FROM gl_source_claims c WHERE c.profile_id=?
                    AND c.source_id=? AND c.start_atomic<? AND c.end_atomic>? AND NOT EXISTS(SELECT 1 FROM gl_source_binding_voids v WHERE v.binding_id=c.binding_id)''',
                    (profile_id,claim['source_id'],claim['end_atomic'],claim['start_atomic'])))
            events.append(dict(event_id=identity,entry_date=day,asset=item['asset'],quantity_msat=quantity,
                wallet=item.get('exchange') or item.get('from_exchange'),categories=categories,claims=claims,
                existing_binding_ids=sorted(bindings),proposal_id=existing['id'] if existing else None))
        events.sort(key=lambda item:(item['entry_date'],item['event_id']))
        return _page(conn,profile_id,dict(kind='events',artifact_id=artifact_id,period_id=period_id),events,limit,cursor)


def list_proposals(conn,profile_id,*,period_id,limit=100,cursor=None):
    with ledger.atomic(conn):
        ledger._period(conn,profile_id,period_id)
        rows=[projection.get_proposal(conn,profile_id,row[0]) for row in conn.execute(
            'SELECT id FROM gl_projection_proposals WHERE profile_id=? AND period_id=? ORDER BY entry_date,id',(profile_id,period_id))]
        return _page(conn,profile_id,dict(kind='proposals',period_id=period_id),rows,limit,cursor)


def list_policies(conn,profile_id,*,period_id,limit=100,cursor=None):
    with ledger.atomic(conn):
        ledger._period(conn,profile_id,period_id)
        policies=[projection.get_policy(conn,profile_id,row[0]) for row in conn.execute(
            "SELECT id FROM gl_projection_policies WHERE profile_id=? AND json_extract(payload_json,'$.period_id')=? ORDER BY created_at,id",(profile_id,period_id))]
        return _page(conn,profile_id,dict(kind='policies',period_id=period_id),policies,limit,cursor)
