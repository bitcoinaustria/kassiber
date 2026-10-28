"""Direct Python producer-composition vectors, separate from raw snapshot facts."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core.custody_snapshot import decode_owned_index
from kassiber.core.ownership_transfers import derive_profile_transfers
from tests.custody_snapshot import to_jsonable

PATH=Path(__file__).parent/'fixtures/custody_pipeline_cases.json'


def books():
    from tests.custody_consolidation_vectors import books as consolidations
    from tests.custody_ownership_vectors import books as ownerships
    from tests.custody_fanout_generator import FANOUT_SCENARIOS, fanout_snapshot
    for prefix,produce in (('consolidation',consolidations),('ownership',ownerships)):
        for case,book,handled in produce():yield prefix+'_'+case,book,handled
    for seed,case in enumerate(FANOUT_SCENARIOS):yield 'fanout_'+case,fanout_snapshot(seed,case=case),[]
    from tests.custody_pipeline_generator import _fanout
    book=_fanout(17,False)
    next(r for r in book['transactions'] if r['direction']=='outbound')['raw_json']='{}'
    yield 'fanout_graphless_source',book,[]


def vectors():
    output=[]
    for case,book,handled in books():
        raw=deepcopy(book)
        for row in raw['transactions']:row.pop('custody_native_fee_normalized',None)
        vector=dict(case=case,input=raw,rows=book['transactions'],handled_ids=handled)
        try:
            result=derive_profile_transfers(book['transactions'],index=decode_owned_index(book['owned_index']),wallet_refs_by_id={str(w['id']):w for w in book['wallets']},already_paired_ids=set(handled))
            vector['expected']={name:{k:sorted(v) if isinstance(v,set) else v for k,v in vars(getattr(result,name)).items()} for name in ('consolidation','ownership','fanout')}
        except Exception as exc:vector['error']=type(exc).__name__
        output.append(to_jsonable(vector,redact_config=False))
    return output


def helper_vectors():
    from kassiber.core.ownership_transfers import OwnershipDeriveResult, ProfileTransferDerivation, _rows_after_derivation
    from tests.custody_interpretation_partial import reference
    def empty():return {k:sorted(v) if isinstance(v,set) else v for k,v in vars(OwnershipDeriveResult()).items()}
    output=[]
    rows=[dict(id='out',amount=5),dict(id='in',amount=5),dict(id='keep',amount=7)]
    for case,changes in (
        ('identity',{}),('drops',dict(dropped_out_ids=['out'],dropped_in_ids=['in'])),
        ('override',dict(out_row_overrides={'keep':dict(id='keep',amount=9)})),
        ('synthetic',dict(synthetic_rows=[dict(id='second'),dict(id='first')])),
        ('drop_before_override',dict(dropped_out_ids=['out'],out_row_overrides={'out':dict(id='other')})),
    ):
        result=empty();result.update(changes)
        derived=OwnershipDeriveResult(**{k:set(v) if k.startswith('dropped_') else v for k,v in result.items()})
        output.append(dict(case=case,op='rows',rows=rows,derivation=result,expected=_rows_after_derivation(rows,derived,sort_key=None)))
    for case,existing in (('residual',{}),('preserve',dict(group_id='original',allow_unclaimed_residual=False)),('no_residual',{})):
        profile={name:empty() for name in ('consolidation','ownership','fanout')}
        profile['consolidation']['derived_pairs']=[dict(source='consolidation')]
        profile['fanout']['derived_pairs']=[dict(source='fanout')]
        profile['ownership']['derived_pairs']=[dict(out=dict(id='synthetic',journal_transaction_id='original'),source='ownership',**existing)]
        if case!='no_residual':profile['ownership']['out_row_overrides']={'original':dict(id='original')}
        pairs=deepcopy(profile['consolidation']['derived_pairs'])
        for pair in profile['ownership']['derived_pairs']:
            pair=dict(pair);source=reference._anchor_id(pair.get('out') or {})
            if profile['ownership']['out_row_overrides'].get(source) is not None:
                pair.setdefault('group_id',f'ownership-event:{source}');pair.setdefault('allow_unclaimed_residual',True)
            pairs.append(pair)
        pairs.extend(profile['fanout']['derived_pairs'])
        output.append(dict(case=case,op='annotate',profile=profile,expected=pairs))
    rows=[dict(id='b',workspace_id='member-b',profile_id='profile-b'),dict(id='a',workspace_id='old-a',profile_id='old-a'),dict(id='a',workspace_id='member-a',profile_id='profile-a')]
    canonical={'events':[{'observation_aliases':[['c','hc'],['b','hb'],['a','ha']]}]}
    for case,excluded,detail,reason,source in (
        ('event',[],{},'ownership_transfer_ambiguous_output','a'),
        ('excluded',['a'],{},'ownership_transfer_ambiguous_output','a'),
        ('excluded_all',['a','b','c'],{},'ownership_transfer_ambiguous_output','a'),
        ('no_event',[],{},'other','missing'),
        ('preserve_detail',[],dict(atomic_event_transaction_ids=['original']),'reason','a'),
        ('default_reason',[],{},None,'a'),
    ):
        profile={name:empty() for name in ('consolidation','ownership','fanout')}
        blocked=dict(row=dict(id=source,workspace_id='blocked',profile_id='blocked'),reason=reason,detail=detail)
        profile['ownership']['blocked_sources']=[blocked,dict(blocked)]
        by_id={r['id']:r for r in rows};members={id:[a[0] for a in sorted(canonical['events'][0]['observation_aliases'])] for id,_ in canonical['events'][0]['observation_aliases']}
        expected={}
        for block in profile['ownership']['blocked_sources']:
            row=block['row'];ids=members.get(row['id'],[row['id']]);why=str(block['reason'] or 'ownership_transfer_unresolved');details=dict(block['detail']);details.setdefault('atomic_event_transaction_ids',ids)
            for id in ids:
                if id in excluded:continue
                member=by_id.get(id,row)
                expected[id,why]=dict(transaction_id=id,workspace_id=member.get('workspace_id'),profile_id=member.get('profile_id'),reason=why,detail_json=json.dumps(details,sort_keys=True))
        output.append(dict(case=case,op='quarantine',rows=rows,canonical=canonical,excluded=excluded,profile=profile,expected=[expected[key] for key in sorted(expected)]))
    return output


if __name__=='__main__':
    for path,values in ((PATH,vectors()),(PATH.with_name('custody_pipeline_helpers.json'),helper_vectors())):
        text=json.dumps(values,indent=2,ensure_ascii=True,sort_keys=True)+'\n'
        if '--write' in sys.argv:path.write_text(text)
        else:assert path.read_text()==text
