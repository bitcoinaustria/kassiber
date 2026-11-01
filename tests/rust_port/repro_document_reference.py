"""Offline reproductions of retained document-import reference quirks."""
from contextlib import closing
import hashlib
from pathlib import Path
import tempfile

from crates.wallets.scripts.generate_document_vectors import base
from kassiber.core import document_import as reference
from tests.test_document_import import _book, _hooks
from kassiber.db import open_db


def main():
    with tempfile.TemporaryDirectory(prefix='document-reference-repro-') as directory:
        root = Path(directory)
        source = root / 'receipt.png'
        source.write_bytes(b'synthetic evidence')
        source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        raw = {**base(), 'id': 'model-row'}
        rows = reference._draft_rows({'rows': [raw]}, threshold=reference.DEFAULT_CONFIDENCE_THRESHOLD,
            source_hash=source_hash, expected_fiat_currency='EUR', allowed_pages=[1])
        assert rows[0]['import_record']['raw_json']['model_row_id'] == 'model-row'
        rebuilt = reference._import_record_from_draft_row(rows[0], source_hash=source_hash,
            expected_fiat_currency='EUR')
        assert 'model_row_id' not in rebuilt['raw_json']
        print('review reconstruction discards the model-supplied row id')
        invalid_page = reference._draft_rows({'rows': [{**raw, 'source_region': {'page': 2}}]},
            threshold=reference.DEFAULT_CONFIDENCE_THRESHOLD, source_hash=source_hash,
            expected_fiat_currency='EUR', allowed_pages=[1])[0]
        assert invalid_page['import_record'] is not None
        assert reference._import_record_from_draft_row(invalid_page, source_hash=source_hash,
            expected_fiat_currency='EUR') is None
        print('a structurally quarantined preview can still carry a convenience import_record; the write validator refuses it')
        with closing(open_db(root / 'data')) as conn:
            _, profile, wallet = _book(conn)
            conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace',?)", (profile['workspace_id'],))
            conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('context_profile',?)", (profile['id'],))
            conn.commit()
            conn.execute('BEGIN')
            outcome = reference.import_document_draft(conn, source_file=str(source),
                data_root=str(root / 'data'), profile=profile, wallet=wallet, rows=rows,
                hooks=_hooks(), expected_source_sha256=source_hash, commit=False)
            assert conn.execute('SELECT count(*) FROM import_batches').fetchone()[0] == 0
            print('document imports record no import run or batch rollback token')
            conn.rollback()
            assert conn.execute('SELECT count(*) FROM transactions').fetchone()[0] == 0
            assert conn.execute('SELECT count(*) FROM attachments').fetchone()[0] == 0
            assert list((root / 'attachments').rglob('*.png'))
            print('caller rollback after a successful commit=False import leaves orphan managed evidence')
            assert len(outcome['attached_evidence']) == 1
            first = reference.import_document_draft(conn, source_file=str(source),
                data_root=str(root / 'data'), profile=profile, wallet=wallet, rows=rows,
                hooks=_hooks(), expected_source_sha256=source_hash)
            second = reference.import_document_draft(conn, source_file=str(source),
                data_root=str(root / 'data'), profile=profile, wallet=wallet, rows=rows,
                hooks=_hooks(), expected_source_sha256=source_hash)
            assert first['imported'] == 1
            assert second['updated'] == 1 and second['unchanged'] == 0
            assert len(second['attached_evidence']) == 1
            print('an identical priced reimport updates the transaction and adds new evidence')
            before = set((root / 'attachments').rglob('*.png'))
            conn.execute("CREATE TRIGGER corrupt_document_digest AFTER INSERT ON attachments BEGIN UPDATE attachments SET sha256='bad' WHERE id=NEW.id; END")
            conn.commit()
            try:
                reference.import_document_draft(conn, source_file=str(source),
                    data_root=str(root / 'data'), profile=profile, wallet=wallet, rows=rows,
                    hooks=_hooks(), expected_source_sha256=source_hash)
            except Exception as exc:
                assert getattr(exc, 'code', None) == 'document_import_source_changed'
            else:
                raise AssertionError('corrupt managed digest was accepted')
            assert len(set((root / 'attachments').rglob('*.png')) - before) == 1
            assert conn.execute('SELECT count(*) FROM attachments').fetchone()[0] == 2
            print('managed-digest verification failure rolls back the database but leaves its untracked copy')
            conn.execute('DROP TRIGGER corrupt_document_digest')
            conn.execute("DELETE FROM settings WHERE key LIKE 'context_%'")
            conn.commit()
            try:
                reference.import_document_draft(conn, source_file=str(source),
                    data_root=str(root / 'data'), profile=profile, wallet=wallet, rows=rows,
                    hooks=_hooks(), expected_source_sha256=source_hash)
            except Exception as exc:
                assert getattr(exc, 'code', None) == 'app_error' and 'No workspace selected' in str(exc)
            else:
                raise AssertionError('explicit profile unexpectedly supplied the attachment scope')
            print('evidence attachment requires active context even with an explicit imported profile')


if __name__ == '__main__':
    main()
