import tempfile
from pathlib import Path

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_links import BASE_SQL, ROOT, outcome


def main():
    scratch = ROOT / 'target/r5-sof-mutations-3/scratch'
    scratch.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='link-precondition-repro-',dir=scratch) as directory:
        conn = open_db(Path(directory)/'data')
        conn.executescript(BASE_SQL)
        conn.commit()
        result = outcome(conn,dict(command='ui.source_funds.links.review',args=dict(link='l-z',state='reviewed',expected_allocation_amount_msat=100001.9)))
        assert result['ok']['state'] == 'reviewed'
        assert result['ok']['allocation_amount_msat'] == 100001
        conn.close()
    print('Python accepts a fractional expected msat value by truncating it before comparison; product code is unchanged.')


if __name__ == '__main__':
    main()
