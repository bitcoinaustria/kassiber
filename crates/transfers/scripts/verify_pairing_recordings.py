"""Compare the pairing stage on restored synthetic session books in target/."""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tests.session_harness.replay import session_dirs
from tests.session_harness.snapshot import BlobStore, restore_tree


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('recordings', nargs='+', type=Path)
    parser.add_argument('--cipher', action='store_true')
    args = parser.parse_args()
    target = ROOT / 'target/r4-pairing/recordings'
    target.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=target) as scratch:
        books = []
        for index, session in enumerate(session_dirs(args.recordings)):
            manifest = json.loads((session / 'manifest.json').read_text())
            state_id = manifest['states']['end']
            if state_id is None:
                continue
            state = json.loads((session / 'states' / f'{state_id}.json').read_text())
            for root_id, tree in state.items():
                if manifest['roots'][root_id]['kind'] not in ('data', 'data_root'):
                    continue
                destination = Path(scratch) / str(index) / root_id
                restore_tree(tree, BlobStore(session / 'blobs'), destination)
                for path in destination.rglob('*.sqlite3'):
                    with path.open('rb') as source:
                        if source.read(16) == b'SQLite format 3\0':
                            connection = sqlite3.connect(path)
                            is_book = connection.execute(
                                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='profiles'"
                            ).fetchone()
                            count = connection.execute('SELECT COUNT(*) FROM profiles').fetchone()[0] if is_book else 0
                            connection.close()
                            if count:
                                books.append(path)
        if not books:
            parser.error('no plaintext books in completed recording data roots')
        command = [sys.executable, str(Path(__file__).with_name('bench_pairing.py')),
                   '--runs', '1', '--out', str(target / ('sqlcipher' if args.cipher else 'sqlite'))]
        for book in books:
            command.extend(['--book', str(book)])
        if args.cipher:
            command.append('--cipher')
        subprocess.run(command, cwd=ROOT, check=True)


if __name__ == '__main__':
    main()
