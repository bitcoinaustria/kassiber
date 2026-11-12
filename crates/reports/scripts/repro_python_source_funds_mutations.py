from pathlib import Path
import tempfile
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import dump, STAMP
from crates.reports.scripts.gen_source_funds_reads import BASE, ROOT


def main():
    scratch = ROOT / 'target/r5-sof-mutations-2/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='source-reference-repro-', dir=scratch) as directory:
        for encrypted in [False, True]:
            conn = open_db(Path(directory) / str(encrypted), passphrase='synthetic-repro-passphrase' if encrypted else None)
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE)
            conn.commit()
            before = dump(conn)
            with patch.object(source_funds.uuid, 'uuid4', return_value='pending-source'), patch.object(source_funds, '_now', return_value=STAMP):
                try:
                    source_funds.create_source(conn, None, None, daemon._source_funds_hooks(), source_type='income', label='Pending origin', attachment_ids=['a-a', 'missing'])
                except AppError as error:
                    assert error.code == 'not_found'
                else:
                    raise AssertionError('missing evidence was accepted')
            assert conn.in_transaction
            assert conn.execute("SELECT count(*) FROM source_funds_sources WHERE id='pending-source'").fetchone()[0] == 1
            assert conn.execute("SELECT count(*) FROM source_funds_source_attachments WHERE source_id='pending-source'").fetchone()[0] == 1
            conn.rollback()
            assert dump(conn) == before
            conn.close()
    print('Python source late-attachment failure retains pending rows on both bindings; caller rollback restores all 114 tables')


if __name__ == '__main__':
    main()
