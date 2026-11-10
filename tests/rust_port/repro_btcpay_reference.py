from contextlib import closing
import sqlite3
from types import SimpleNamespace

from kassiber.core.imports import apply_btcpay_metadata, normalize_import_record
from kassiber.importers import normalize_btcpay_record


def main():
    txid = 'True'
    source = normalize_btcpay_record({'TransactionId':True, 'Timestamp':'2025-01-02', 'Amount':'1 BTC', 'Comment':'receipt'})
    normalized = normalize_import_record(source, 'file:btcpay_json')
    assert normalized['external_id'] == txid
    with closing(sqlite3.connect(':memory:')) as conn:
        conn.row_factory = sqlite3.Row
        conn.execute('CREATE TABLE transactions(id TEXT,profile_id TEXT,wallet_id TEXT,external_id TEXT,note TEXT,created_at TEXT)')
        conn.execute("INSERT INTO transactions VALUES('tx','p','w',?,NULL,'2025-01-02')", (normalized['external_id'],))
        conn.execute("INSERT INTO transactions VALUES('numeric','p','w','1',NULL,'2025-01-02')")
        # 464f82094: metadata binds to str(txid), the identity observations
        # store, so a boolean identity finds its own row and never the
        # numeric neighbour SQLite would bind True to.
        outcome = apply_btcpay_metadata(conn, {'id':'p','workspace_id':'ws'}, {'id':'w'}, [source], SimpleNamespace())
        assert outcome['btcpay_notes_set'] == 1
        assert conn.execute("SELECT note FROM transactions WHERE id='tx'").fetchone()[0] == 'receipt'
        assert conn.execute("SELECT note FROM transactions WHERE id='numeric'").fetchone()[0] is None
    print('BTCPay reference behaviour: boolean identity metadata binds to its stored text identity only')


if __name__ == '__main__':
    main()
