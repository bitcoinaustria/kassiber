"""Reproduce the reference's committed batch before response projection fails."""
from pathlib import Path
import tempfile

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_batch import BASE_SQL, ROOT, cases, outcome


def main():
    scratch=ROOT/'target/r5-sof-mutations-4/scratch'
    scratch.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='batch-projection-repro-',dir=scratch) as directory:
        for encrypted in [False,True]:
            conn=open_db(Path(directory)/str(encrypted),passphrase='synthetic-passphrase' if encrypted else None)
            conn.executescript(BASE_SQL)
            case=next(c for c in cases(conn) if c['name']=='two-current')
            conn.executescript(case['sql'])
            conn.execute("""CREATE TRIGGER refuse_projection AFTER UPDATE ON source_funds_links
                WHEN NEW.id='l-second' BEGIN UPDATE source_funds_links
                SET allocation_amount='synthetic malformed amount' WHERE id='l-second'; END;""")
            result=outcome(conn,case)
            assert result['error']['code']=='internal_error'
            assert [row[0] for row in conn.execute("SELECT state FROM source_funds_links WHERE id IN ('l-z','l-second')")] == ['reviewed','reviewed']
            assert conn.execute("SELECT allocation_amount FROM source_funds_links WHERE id='l-second'").fetchone()[0]=='synthetic malformed amount'
            conn.close()
    print('On SQLite and SQLCipher Python commits both batch reviews before a failing response projection. Native admission rolls back this path; Python remains unchanged.')


if __name__=='__main__':main()
