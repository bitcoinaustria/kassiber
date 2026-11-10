"""Python reference repro: a rolled-back save hides why it failed.

``save_case_snapshot`` (c0a015de2) wraps both inserts in the
``source_funds_case_snapshot`` savepoint and, on any error, runs ``ROLLBACK TO``
it. A refusal that ends the whole transaction (a trigger's
``RAISE(ROLLBACK, ...)``) has already removed the savepoint, so that cleanup
raises ``no such savepoint`` and the caller sees it instead of the refusal.
The book is unchanged either way. The native port reports the original
refusal (docs/reference/rust-shell.md, source-of-funds saves).
"""
from pathlib import Path
import sqlite3
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def reproduce():
    from kassiber.core import source_funds

    conn = sqlite3.connect(":memory:")
    conn.executescript(
        "CREATE TABLE source_funds_cases(id, workspace_id, profile_id, target_transaction_id,"
        " target_external_id, target_amount, asset, label, reveal_mode, status, snapshot_hash,"
        " snapshot_json, created_at, updated_at, recipient_id, recipient_label_snapshot,"
        " recipient_kind_snapshot, recipient_reveal_mode_snapshot);"
        "CREATE TABLE source_funds_snapshots(id, case_id, snapshot_hash, snapshot_json, created_at);"
        "CREATE TRIGGER refuse_save BEFORE INSERT ON source_funds_cases"
        " BEGIN SELECT RAISE(ROLLBACK, 'synthetic late refusal'); END;"
    )
    conn.execute("BEGIN IMMEDIATE")
    try:
        source_funds.save_case_snapshot(conn, "ws", "p", "tx", 1, "BTC", "standard", "ready", {})
    except sqlite3.Error as error:
        return str(error)
    finally:
        conn.close()
    return None


if __name__ == "__main__":
    message = reproduce()
    print(message)
    # The reference reports its own cleanup failure, not the refusal.
    sys.exit(0 if message == "no such savepoint: source_funds_case_snapshot" else 1)
