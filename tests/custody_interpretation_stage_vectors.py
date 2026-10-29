"""Complete CPython compilation, wrapper and admission-order vectors."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from tests.custody_interpretation_stage import python_stage
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / "fixtures/custody_interpretation_stages.json"
WRAPPERS = PATH.with_name("custody_interpretation_wrappers.json")


def expected(book):
    try: return dict(expected=to_jsonable(python_stage(book), redact_config=False))
    except Exception as exc: return dict(error=getattr(exc,"code",type(exc).__name__))


def vectors():
    from tests.custody_interpretation_vectors import books
    return [dict(case=case,**expected(book)) for case,book in books()]


def wrappers():
    from tests.custody_snapshot_generator import snapshot
    from tests.custody_channel_generator import channel_snapshot
    result=[]
    for i,ids in enumerate(('[]','["first","first",null,true,4,1.25,{},[1],"\\ud800"]','{"second":1,"first":2}','"ba🐍"','null','1','true','bad',None,{},[1],0,'')):
        for j,details in enumerate(('{}','null','[1]','bad','{"float":NaN,"text":"\\ud800"}',None,{})):
            book=snapshot(0)
            book['migration_issues']=[dict(id='migration',workspace_id='ws',profile_id=book['profile']['id'],legacy_table='pairs',legacy_source_id='old',issue_code='ambiguous',transaction_ids_json=ids,details_json=details)]
            result.append(dict(case=f'migration_{i}_{j}',input=book,**expected(book)))
    for key in ('transaction_ids_json','id','legacy_table','legacy_source_id','issue_code','details_json','workspace_id','profile_id'):
        book=deepcopy(result[7]['input']);book['migration_issues'][0].pop(key)
        result.append(dict(case='missing_'+key,input=book,**expected(book)))
    book=channel_snapshot(0,case='strict_lnd')
    book['channel_records'][0]['amount_msat']='bad'
    from tests.custody_reconciliation_generator import snapshot as route
    c=route(0)['components'][0];c['legs'][0]['amount_msat']=[1]
    book['components']=[c]
    result.append(dict(case='channel_before_components',input=book,**expected(book)))
    book=deepcopy(result[0]['input']);book['migration_issues'][0]['transaction_ids_json']='null';book['migration_issues'][0].pop('details_json')
    result.append(dict(case='migration_details_before_id_iteration',input=book,**expected(book)))
    book=deepcopy(result[0]['input']);book['components']=[c]
    book['migration_issues'][0].pop('details_json')
    result.append(dict(case='components_before_migration',input=book,**expected(book)))
    book=deepcopy(result[0]['input']);book['migration_issues'][0]['transaction_ids_json']='null'
    book['transactions'][0].pop('id')
    result.append(dict(case='migration_before_evidence',input=book,**expected(book)))
    book=deepcopy(result[-4]['input']);book['profile']['tax_country']='unsupported'
    result.append(dict(case='tax_gate_before_channel',input=book,**expected(book)))
    from tests.custody_safety_generator import safety_snapshot
    book=safety_snapshot(0,case='pending_pair')
    first=deepcopy(result[7]['input']['migration_issues'][0])
    first['transaction_ids_json']='["z","a","z"]'
    book['migration_issues']=[first,dict(first,id='second',transaction_ids_json='["last"]')]
    result.append(dict(case='migration_append_and_sort',input=book,**expected(book)))
    from tests.custody_snapshot_generator import SCENARIOS
    book=snapshot(SCENARIOS.index('zero_delta'))
    stage=expected(book)
    assert stage['expected']['non_event_transaction_ids']
    result.append(dict(case='native_zero_delta_union',input=book,**stage))
    return result


if __name__=='__main__':
    for path,actual in ((PATH,vectors()),(WRAPPERS,wrappers())):
        contents=json.dumps(actual,ensure_ascii=True,separators=(',',':'))+'\n'
        if '--check' in sys.argv: assert path.read_text()==contents,path
        else: path.write_text(contents)
