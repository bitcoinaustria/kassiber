"""Importer regression recorded by the Rust port."""

import json

from kassiber.cli import handlers
from kassiber.core import import_batches, imports
from kassiber.db import open_db
from tests.test_custody_authored_migration import _scope


def test_wasabi_file_import_has_a_rollback_batch(tmp_path):
    path = tmp_path / "wasabi.json"
    path.write_text(json.dumps([{"datetime": "2025-01-01", "tx": "ab" * 32, "amount": 1}]))
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        conn.commit()
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id='btc'").fetchone()
        outcome = imports.import_file_into_wallet(conn, profile, wallet, str(path), "wasabi_bundle", handlers._import_coordinator_hooks())
        batch_id = outcome["import_batch_id"]
        batch = conn.execute("SELECT * FROM import_batches WHERE id=?", (batch_id,)).fetchone()
        assert batch["source_filename"] == "wasabi.json"
        assert batch["source_format"] == "wasabi_bundle"
        assert batch["rows_inserted"] == 1
        assert import_batches.plan_rollback(conn, profile["id"], batch_id)["transactions_to_delete"] == 1
        assert imports.import_file_into_wallet(conn, profile, wallet, str(path), "wasabi_bundle", handlers._import_coordinator_hooks())["imported"] == 0
        assert conn.execute("SELECT COUNT(*) FROM import_batches").fetchone()[0] == 1
    finally:
        conn.close()
