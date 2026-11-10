import json
from pathlib import Path
import tempfile
from unittest.mock import patch

from kassiber.core import source_funds
from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_cases import cases, ROOT, STAMP
from crates.reports.scripts.gen_source_funds_reads import BASE


def main():
    scratch = ROOT / 'target/r5-sof-mutations/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='case-reference-repro-', dir=scratch) as directory:
        conn = open_db(Path(directory) / 'data')
        conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE)
        case = next(case for case in cases() if case['name'] == 'source_funds_snapshots-ABORT')
        conn.executescript(case['sql'])
        with patch.object(source_funds.uuid, 'uuid4', side_effect=['new-case', 'new-snapshot']), patch.object(source_funds, '_now', return_value=STAMP):
            try:
                source_funds.save_case_snapshot(conn, **case['args'])
            except Exception as error:
                assert str(error) == 'synthetic late refusal'
            else:
                raise AssertionError('late failure did not occur')
        assert conn.in_transaction
        assert conn.execute("SELECT count(*) FROM source_funds_cases WHERE id='new-case'").fetchone()[0] == 1
        assert conn.execute("SELECT count(*) FROM source_funds_snapshots WHERE case_id='new-case'").fetchone()[0] == 0
        conn.rollback()
        assert conn.execute("SELECT count(*) FROM source_funds_cases WHERE id='new-case'").fetchone()[0] == 0
        conn.close()
    print(json.dumps(dict(assertions=5, reference_quirk='snapshot insert refusal leaves a pending case until caller rollback')))


if __name__ == '__main__':
    main()
