"""Reproduce the unchanged reference's truncated classified-count text."""
from pathlib import Path
import tempfile

from kassiber import daemon
from kassiber.core.source_funds_coverage import compute_coverage, coverage_summary_text
from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_review import BASE_SQL, SCRATCH


def main():
    SCRATCH.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data')
        conn.executescript(BASE_SQL)
        result=compute_coverage(conn,None,None,daemon._source_funds_hooks(),max_transactions=0)
        total=result['totals']['tx_count']
        skipped=result['truncation']['not_classified_count']
        assert total==skipped and total>0
        line=coverage_summary_text(result)[0]
        assert line==f'Coverage truncated: classified {total} of {total} inbound tx; {skipped} not classified.'
        conn.close()
        print(line)
        print('Reference bug preserved: no transaction was classified, but the summary claims every inbound row was classified.')


if __name__=='__main__':main()
