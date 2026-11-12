"""Python reference repro: a missing inbox acknowledgement leaves a transaction open.

The bare UPDATE triggers the driver's implicit BEGIN; the not-found error is
raised before any commit, so the caller's connection remains in a write
transaction. The native port preserves this observable state.
"""
from pathlib import Path
import sqlite3
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def reproduce():
    from kassiber.core.chain_analysis_watches import acknowledge
    from kassiber.errors import AppError
    conn = sqlite3.connect(':memory:')
    conn.executescript((ROOT/'crates/reports/tests/data/privacy_hygiene_seed.sql').read_text())
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text())
    try:
        acknowledge(conn, 'p', {'id': 'absent'})
    except AppError as error:
        code = error.code
    else:
        code = None
    try:
        return {'acknowledged_missing': code, 'in_transaction': conn.in_transaction}
    finally:
        conn.close()


if __name__ == '__main__':
    print(reproduce())
