"""BTCPay metadata must use the same external identity as its observation."""

import sqlite3
from types import SimpleNamespace

from kassiber.core.imports import apply_btcpay_metadata, normalize_import_record
from kassiber.importers import normalize_btcpay_record


def test_boolean_btcpay_identity_never_targets_a_numeric_neighbor():
    source = normalize_btcpay_record({"TransactionId": True, "Timestamp": "2025-01-02", "Amount": "1 BTC", "Comment": "receipt"})
    normalized = normalize_import_record(source, "file:btcpay_json")
    with sqlite3.connect(":memory:") as conn:
        conn.row_factory = sqlite3.Row
        conn.execute("CREATE TABLE transactions(id TEXT,profile_id TEXT,wallet_id TEXT,external_id TEXT,note TEXT,created_at TEXT)")
        conn.execute("INSERT INTO transactions VALUES('actual','p','w',?,NULL,'2025-01-02')", (normalized["external_id"],))
        conn.execute("INSERT INTO transactions VALUES('neighbor','p','w','1',NULL,'2025-01-02')")
        outcome = apply_btcpay_metadata(conn, {"id": "p", "workspace_id": "ws"}, {"id": "w"}, [source], SimpleNamespace())
        assert outcome["btcpay_notes_set"] == 1
        assert conn.execute("SELECT note FROM transactions WHERE id='actual'").fetchone()[0] == "receipt"
        assert conn.execute("SELECT note FROM transactions WHERE id='neighbor'").fetchone()[0] is None
