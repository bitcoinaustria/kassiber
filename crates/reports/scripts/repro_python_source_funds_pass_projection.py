import json
from pathlib import Path
import tempfile

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_passes import outcome, ROOT


def main():
    scratch=ROOT/'target/r5-sof-mutations-5/projection-repro';scratch.mkdir(parents=True,exist_ok=True)
    cases=json.loads((ROOT/'crates/reports/tests/data/source_funds_passes.json').read_text())
    results=[]
    for encrypted in [False,True]:
        for name,event in [('suggest-provider-default','INSERT'),('assemble-structural-current-1','UPDATE')]:
            case=next(c for c in cases if c['name']==name)
            with tempfile.TemporaryDirectory(dir=scratch) as directory:
                conn=open_db(Path(directory)/'data',passphrase='synthetic-passes-passphrase' if encrypted else None)
                conn.executescript(case['sql'])
                conn.execute(f"CREATE TRIGGER corrupt_response AFTER {event} ON source_funds_links BEGIN UPDATE source_funds_links SET allocation_amount='bad' WHERE id=NEW.id; END;")
                conn.commit();result=outcome(conn,case)
                assert 'error' in result
                persisted=[dict(row) for row in conn.execute('SELECT id,state,allocation_amount FROM source_funds_links')]
                assert persisted and any(row['allocation_amount']=='bad' for row in persisted)
                results.append(dict(binding='sqlcipher' if encrypted else 'sqlite',command=case['command'],error=result['error'],persisted=persisted))
                conn.close()
    print(json.dumps(results,indent=2))


if __name__=='__main__':main()
