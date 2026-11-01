from pathlib import Path
import tempfile

from kassiber.core import source_funds_recipients as recipients
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_reads import BASE, ROOT


def main():
    scratch = ROOT / 'target/r5-sof-mutations-2/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='recipient-constraint-repro-', dir=scratch) as directory:
        for encrypted in [False, True]:
            conn = open_db(Path(directory) / str(encrypted), passphrase='synthetic-recipient-passphrase' if encrypted else None)
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text()+BASE)
            conn.commit()
            try:
                recipients.create_recipient(conn, 'ws', 'p', label='alpha', kind='bank')
            except AppError as error:
                assert not encrypted
                assert error.code == 'validation'
                assert str(error) == "A recipient labelled 'alpha' already exists in this profile."
            except Exception as error:
                from sqlcipher3 import dbapi2 as cipher
                assert encrypted
                assert isinstance(error, cipher.IntegrityError)
                assert str(error) == 'UNIQUE constraint failed: source_funds_recipients.profile_id, source_funds_recipients.label'
            else:
                raise AssertionError('duplicate recipient was accepted')
            conn.rollback()
            assert conn.execute("SELECT count(*) FROM source_funds_recipients WHERE profile_id='p' AND label='alpha'").fetchone()[0] == 1
            conn.close()
    print('Python catches sqlite3 recipient constraint errors but not SQLCipher constraint errors; product code remains unchanged')


if __name__ == '__main__':
    main()
