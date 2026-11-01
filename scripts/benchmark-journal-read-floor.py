import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from kassiber.db import open_db
from tests.rust_port.journal_persistence_oracle import dump


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def snapshot_hash(conn):
    return hashlib.sha256(json.dumps(dump(conn), sort_keys=True).encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--books', type=Path, nargs='+', required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--probe', type=Path, default=Path('target/release/examples/journal-read-floor'))
    args = parser.parse_args()
    target = Path('target').resolve()
    out = args.out.resolve()
    if not out.is_relative_to(target):
        parser.error('--out must be under worktree target/')
    out.mkdir(parents=True, exist_ok=True)
    record = dict(
        probe_sha256=digest(args.probe),
        sources={str(p): digest(p) for p in [Path(__file__), Path('crates/journals/examples/journal-read-floor.rs')]},
        method='Five scans on independent newly opened connections, matching native benchmark cache state; independent CPU minimum. Prepared SELECTs, borrowed ValueRef for every retained column, one black_box touch per scalar, no hashing, conversion, allocation or byte comparison. SELECT iteration and COMMIT timed. Complete dumps verified outside timing.',
        cells=[],
    )
    for book in args.books:
        if not book.resolve().is_relative_to(target):
            parser.error('--books must be under worktree target/')
        manifest = json.loads((book / 'bench-book.json').read_text())
        for name, expected in manifest['csv_sha256'].items():
            assert digest(book / name) == expected, (book, name)
        for cipher in (False, True):
            root = out / (book.name + ('-cipher' if cipher else '-sqlite'))
            root.mkdir(exist_ok=True)
            path = root / 'kassiber.sqlite3'
            shutil.copyfile(book / 'data/kassiber.sqlite3', path)
            key = 'synthetic-test-only' if cipher else None
            if cipher:
                from sqlcipher3 import dbapi2 as sqlcipher
                encrypted = root / 'encrypted.sqlite3'
                c = sqlcipher.connect(path)
                version = c.execute('PRAGMA user_version').fetchone()[0]
                c.execute('ATTACH DATABASE ? AS encrypted KEY ?', (str(encrypted), key))
                c.execute("SELECT sqlcipher_export('encrypted')")
                c.execute(f'PRAGMA encrypted.user_version={int(version)}')
                c.execute('DETACH DATABASE encrypted')
                c.close()
                path.unlink()
                encrypted.replace(path)
            conn = open_db(root, passphrase=key)
            before = snapshot_hash(conn)
            counts = {table: conn.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                      for table in ('journal_entries', 'journal_quantity_postings')}
            policy = {name: conn.execute(f'PRAGMA {name}').fetchone()[0]
                      for name in ('journal_mode', 'synchronous')}
            conn.close()
            request = root / 'request.json'
            request.write_text(json.dumps(dict(database=str(path), passphrase=key)))
            result = subprocess.run([str(args.probe.resolve()), str(request)],
                                    check=True, text=True, capture_output=True)
            cell = json.loads(result.stdout)
            for trial in cell['trials']:
                assert trial['entries']['rows'] == counts['journal_entries']
                assert trial['postings']['rows'] == counts['journal_quantity_postings']
                assert trial['entries']['cells'] == counts['journal_entries'] * 29
                assert trial['postings']['cells'] == counts['journal_quantity_postings'] * 12
            conn = open_db(root, passphrase=key)
            assert snapshot_hash(conn) == before
            assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            assert not conn.execute('PRAGMA foreign_key_check').fetchall()
            conn.close()
            cell.update(book=manifest, policy=policy, logical_dump_sha256=before,
                        best_cpu=min(t['cpu_seconds'] for t in cell['trials']))
            record['cells'].append(cell)
            (out / 'summary.json').write_text(json.dumps(record, indent=2) + '\n')
            print(root.name, cell['best_cpu'], flush=True)
            shutil.rmtree(root)


if __name__ == '__main__':
    main()
