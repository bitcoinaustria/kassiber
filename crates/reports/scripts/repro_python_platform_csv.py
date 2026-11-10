import csv
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from crates.reports.scripts.gen_platform_csv_vectors import reference
from kassiber.db import open_db


def main():
    scratch=ROOT/'target/r5-platform/scratch'
    scratch.mkdir(parents=True,exist_ok=True)
    results={}
    cases=json.loads((ROOT/'crates/reports/tests/data/platform_csv.json').read_text())
    for name in ['overflow-movement','empty-date','empty-exact','short-year']:
        case=next(c for c in cases if c['name']=='cointracking-'+name)
        with tempfile.TemporaryDirectory(dir=scratch) as raw:
            root=Path(raw)
            conn=open_db(root/'data')
            conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+case['sql'])
            path=root/'platform.csv'
            payload=reference(conn,path,json.loads(case['args_json']))
            if 'error' in payload:
                results[name]=payload['error']['message']
            else:
                with path.open(newline='') as handle:
                    rows=list(csv.DictReader(handle))
                results[name]=rows[0]['Date'] if name=='short-year' else next(r['Buy Amount'] for r in rows if r['Tx-ID']=='out')
            conn.close()
    assert results['overflow-movement']=='integer overflow'
    assert results['empty-date']=="Invalid isoformat string: ''"
    assert results['empty-exact']=='0'
    assert results['short-year']=='1-01-01 01:02:03'
    print(json.dumps(results,sort_keys=True))


if __name__=='__main__':
    main()
