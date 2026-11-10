import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace

from kassiber.core import runtime
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_links import BASE_SQL, ROOT, outcome


def main():
    scratch = ROOT / 'target/r5-sof-mutations-3/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='projection-commit-repro-', dir=scratch) as directory:
        for encrypted in [False, True]:
            for family, identity, column in [('sources','s-z','amount'),
                                              ('links','l-z','allocation_amount')]:
                conn = open_db(Path(directory)/f'{encrypted}-{family}',
                               passphrase='synthetic-passphrase' if encrypted else None)
                conn.executescript(BASE_SQL)
                conn.execute(f'UPDATE source_funds_{family} SET {column}=? WHERE id=?',
                             ('api_key=synthetic-projection-secret', identity))
                conn.commit()
                result = outcome(conn, dict(command=f'ui.source_funds.{family}.attach',
                    args={('source' if family == 'sources' else 'link'):identity,
                          'attachment_id':'a-a'}))
                assert result['error']['code'] == 'internal_error'
                table = f'source_funds_{"source" if family == "sources" else "link"}_attachments'
                key = 'source_id' if family == 'sources' else 'link_id'
                assert conn.execute(f'SELECT COUNT(*) FROM {table} WHERE {key}=? AND attachment_id=?',
                                    (identity,'a-a')).fetchone()[0] == 1
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    runtime.emit_error(SimpleNamespace(format='json', output=None),
                        AppError(result['error']['message'], code='internal_error'))
                assert 'synthetic-projection-secret' in json.loads(output.getvalue())['error']['message']
                conn.close()
    print('Python commits evidence associations before a failing response projection; its CLI error message retains a synthetic credential assignment. Product code is unchanged.')


if __name__ == '__main__':
    main()
